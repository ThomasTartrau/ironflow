//! Parallel step wave for [`WorkflowContext`].

use std::collections::HashMap;

use chrono::Utc;
use rust_decimal::Decimal;
use serde_json::to_value;
use tokio::task::{Id, JoinSet};
use tokio::time::timeout;
use tracing::{error, info};
use uuid::Uuid;

use ironflow_store::models::{NewStep, StepStatus, StepUpdate, step_trace_id};

use crate::budget::step_budget_usd;
use crate::config::StepConfig;
use crate::context::WorkflowContext;
use crate::context::failure::{
    allowed_failure_output, extract_debug_messages_from_error, extract_partial_usage_from_error,
    extract_raw_response_from_error,
};
use crate::error::EngineError;
use crate::executor::{
    ParallelStepResult, StepArtifacts, StepOutput, StepResult, execute_step_config_intercepted,
};
use crate::guard::WorkflowRejection;
use crate::log_sender::StepLogSender;
use crate::notify::{WorkflowAgentStepTokensUsedEvent, WorkflowEvent};
use crate::plan::{lock_plan, planned_output};

impl WorkflowContext {
    /// Execute multiple steps concurrently (wait-all model).
    ///
    /// All steps in the batch execute in parallel via `tokio::JoinSet`.
    /// Each step is recorded with the same `position` (execution wave).
    /// Dependencies on previous steps are recorded automatically.
    ///
    /// When `fail_fast` is true, remaining steps are aborted on the first
    /// failure. When false, all steps run to completion and the first
    /// error is returned.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] if any step fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::context::WorkflowContext;
    /// use ironflow_engine::config::{StepConfig, ShellConfig};
    /// use ironflow_engine::error::EngineError;
    ///
    /// # async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    /// let results = ctx.parallel(
    ///     vec![
    ///         ("test-unit", StepConfig::Shell(ShellConfig::new("cargo test --lib"))),
    ///         ("lint", StepConfig::Shell(ShellConfig::new("cargo clippy"))),
    ///     ],
    ///     true,
    /// ).await?;
    ///
    /// for r in &results {
    ///     println!("{}: {:?}", r.name, r.output.output);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn parallel(
        &mut self,
        steps: Vec<(&str, StepConfig)>,
        fail_fast: bool,
    ) -> Result<Vec<ParallelStepResult>, EngineError> {
        if steps.is_empty() {
            return Ok(Vec::new());
        }

        // Plan mode: record the whole wave under one parallel group and return
        // synthetic outputs. No step record is created and nothing runs.
        if let Some(plan) = self.plan().cloned() {
            self.position += 1;
            let mut results = Vec::with_capacity(steps.len());
            let mut names = Vec::with_capacity(steps.len());
            {
                let mut recorder = lock_plan(&plan);
                let group = recorder.next_group();
                for (name, config) in &steps {
                    let wave = Some(group.clone());
                    if !recorder.record(name, config.kind(), &self.workflow_name, wave) {
                        break;
                    }
                    names.push((*name).to_string());
                    let estimate = recorder.estimate_for(name);
                    let mut output = planned_output(config, estimate);
                    output.artifacts = StepArtifacts::new(name, None, config.declared_outputs());
                    results.push(ParallelStepResult {
                        name: (*name).to_string(),
                        output,
                        step_id: Uuid::now_v7(),
                    });
                }
                recorder.set_last(names);
            }
            return Ok(results);
        }

        // Guard timeout: checked before launching the wave.
        self.check_guard_timeout()?;

        // Cost cap: the whole wave is charged at once. Refused before any step
        // record is created, so nothing in the wave starts.
        let wave_budget: Decimal = steps
            .iter()
            .filter_map(|(_, config)| match config {
                StepConfig::Agent(agent_config) => Some(agent_config.max_budget_usd),
                _ => None,
            })
            .map(step_budget_usd)
            .sum();
        self.check_run_budget(wave_budget)?;

        let wave_position = self.position;
        self.position += 1;

        let now = Utc::now();
        let mut step_records: Vec<(Uuid, Uuid, String, StepConfig)> =
            Vec::with_capacity(steps.len());

        for (name, config) in &steps {
            let kind = config.kind();
            let trace_id = step_trace_id(self.run_id, name, wave_position);
            let step = self
                .store
                .create_step(NewStep {
                    run_id: self.run_id,
                    trace_id,
                    name: name.to_string(),
                    kind,
                    position: wave_position,
                    input: Some(to_value(config)?),
                    is_error_handler: false,
                })
                .await?;

            self.start_step(step.id, now).await?;

            // Inputs are materialized before any step in the wave starts, so a
            // missing one fails the wave rather than a half-run command.
            if let Err(err) = self.prepare_step_inputs(config, wave_position).await {
                self.fail_step(step.id, &err).await;
                if !config.allow_failure() {
                    return Err(err);
                }
                self.has_allowed_failure = true;
                info!(
                    run_id = %self.run_id,
                    step = %name,
                    error = %err,
                    "parallel step input preparation failed but allow_failure is set, skipping"
                );
                continue;
            }

            let mut config_with_trace = config.clone();
            let step_trace = self.trace_context.child();
            match config_with_trace {
                StepConfig::Agent(ref mut agent_config) => {
                    agent_config.trace_context = Some(step_trace);
                }
                StepConfig::Http(ref mut http_config) => {
                    http_config.trace_context = Some(step_trace);
                }
                _ => {}
            }
            step_records.push((step.id, trace_id, name.to_string(), config_with_trace));
        }

        let mut join_set = JoinSet::new();
        let mut task_index: HashMap<Id, usize> = HashMap::new();
        let parallel_timeout = self.guard_remaining_timeout();
        for (idx, (step_id, _trace_id, step_name, config)) in step_records.iter().enumerate() {
            let provider = self.provider.clone();
            // Each task owns its own handle: `intercept` is synchronous, so no
            // borrow of the context is held across an await point.
            let interceptor = self.interceptor.clone();
            let config = config.clone();
            let step_log_sender = self
                .log_sender
                .as_ref()
                .map(|s| StepLogSender::new(s.clone(), self.run_id, *step_id, step_name.clone()));
            let handle = join_set.spawn(async move {
                let result = match parallel_timeout {
                    Some(dur) => {
                        match timeout(
                            dur,
                            execute_step_config_intercepted(
                                &config,
                                &provider,
                                step_log_sender,
                                interceptor.as_ref(),
                            ),
                        )
                        .await
                        {
                            Ok(r) => r,
                            Err(_elapsed) => {
                                Err(EngineError::from(WorkflowRejection::WorkflowTimeout {
                                    elapsed_secs: 0,
                                    max: 0,
                                }))
                            }
                        }
                    }
                    None => {
                        execute_step_config_intercepted(
                            &config,
                            &provider,
                            step_log_sender,
                            interceptor.as_ref(),
                        )
                        .await
                    }
                };
                (idx, result)
            });
            task_index.insert(handle.id(), idx);
        }

        // JoinSet returns in completion order; indexed_results restores input order.
        let mut indexed_results: Vec<Option<Result<StepOutput, String>>> =
            vec![None; step_records.len()];
        let mut first_error: Option<EngineError> = None;

        while let Some(join_result) = join_set.join_next().await {
            let (idx, step_result) = match join_result {
                Ok(r) => r,
                Err(e) => {
                    let error_msg = format!("join error: {e}");
                    if let Some(&idx) = task_index.get(&e.id()) {
                        let (step_id, _, step_name, _) = &step_records[idx];
                        let completed_at = Utc::now();
                        error!(
                            run_id = %self.run_id,
                            step = %step_name,
                            error = %error_msg,
                            "parallel step panicked or was cancelled"
                        );
                        if let Err(store_err) = self
                            .store
                            .update_step(
                                *step_id,
                                StepUpdate {
                                    status: Some(StepStatus::Failed),
                                    error: Some(error_msg.clone()),
                                    completed_at: Some(completed_at),
                                    ..StepUpdate::default()
                                },
                            )
                            .await
                        {
                            error!(
                                run_id = %self.run_id,
                                step_id = %step_id,
                                error = %store_err,
                                "failed to persist JoinError for step"
                            );
                        }
                        indexed_results[idx] = Some(Err(error_msg.clone()));
                    }
                    if first_error.is_none() {
                        first_error = Some(EngineError::StepConfig(error_msg));
                    }
                    if fail_fast {
                        join_set.abort_all();
                    }
                    continue;
                }
            };

            let (step_id, step_trace, step_name, step_config) = &step_records[idx];
            let completed_at = Utc::now();

            if let Err(err) = self
                .store_step_outputs(step_config, *step_id, step_name, step_result.is_ok())
                .await
            {
                self.fail_step(*step_id, &err).await;
                indexed_results[idx] = Some(Err(err.to_string()));
                if first_error.is_none() {
                    first_error = Some(err);
                }
                if fail_fast {
                    join_set.abort_all();
                }
                continue;
            }

            match step_result {
                Ok(output) => {
                    self.total_cost_usd += output.cost_usd;
                    self.total_duration_ms += output.duration_ms;

                    // Record token usage in the guard for agent steps.
                    if matches!(step_config, StepConfig::Agent(_)) {
                        let tokens = output
                            .input_tokens
                            .unwrap_or(0)
                            .saturating_add(output.output_tokens.unwrap_or(0));
                        if tokens > 0
                            && let Err(guard_err) = self.guard_record_tokens(tokens)
                        {
                            if first_error.is_none() {
                                first_error = Some(guard_err);
                            }
                            if fail_fast {
                                join_set.abort_all();
                            }
                        }
                    }

                    let debug_messages_json = output.debug_messages_json();

                    self.store
                        .update_step(
                            *step_id,
                            StepUpdate {
                                status: Some(StepStatus::Completed),
                                output: Some(output.output.clone()),
                                duration_ms: Some(output.duration_ms),
                                cost_usd: Some(output.cost_usd),
                                input_tokens: output.input_tokens,
                                output_tokens: output.output_tokens,
                                completed_at: Some(completed_at),
                                debug_messages: debug_messages_json,
                                ..StepUpdate::default()
                            },
                        )
                        .await?;

                    self.step_results.push(StepResult::from_success(
                        *step_trace,
                        step_name,
                        &output,
                    ));

                    if let Some(ref bus) = self.event_bus
                        && matches!(step_config, StepConfig::Agent(_))
                    {
                        let tokens = output
                            .input_tokens
                            .unwrap_or(0)
                            .saturating_add(output.output_tokens.unwrap_or(0));
                        bus.publish(
                            self.run_id,
                            WorkflowEvent::AgentStepTokensUsed(WorkflowAgentStepTokensUsedEvent {
                                step_name: step_name.clone(),
                                tokens,
                                cost_usd: output.cost_usd,
                            }),
                        );
                    }

                    info!(
                        run_id = %self.run_id,
                        step = %step_name,
                        trace_id = %step_trace,
                        duration_ms = output.duration_ms,
                        "parallel step completed"
                    );

                    indexed_results[idx] = Some(Ok(output));
                }
                Err(err) => {
                    let err_msg = err.to_string();
                    let debug_messages_json = extract_debug_messages_from_error(&err);
                    let partial = extract_partial_usage_from_error(&err);
                    let raw_response_output = extract_raw_response_from_error(&err);

                    if let Some(ref usage) = partial {
                        if let Some(cost) = usage.cost_usd {
                            self.total_cost_usd += cost;
                        }
                        if let Some(dur) = usage.duration_ms {
                            self.total_duration_ms += dur;
                        }
                    }

                    if let Err(store_err) = self
                        .store
                        .update_step(
                            *step_id,
                            StepUpdate {
                                status: Some(StepStatus::Failed),
                                error: Some(err_msg.clone()),
                                output: raw_response_output.clone(),
                                completed_at: Some(completed_at),
                                debug_messages: debug_messages_json,
                                duration_ms: partial.as_ref().and_then(|p| p.duration_ms),
                                cost_usd: partial.as_ref().and_then(|p| p.cost_usd),
                                input_tokens: partial.as_ref().and_then(|p| p.input_tokens),
                                output_tokens: partial.as_ref().and_then(|p| p.output_tokens),
                                ..StepUpdate::default()
                            },
                        )
                        .await
                    {
                        error!(
                            step_id = %step_id,
                            error = %store_err,
                            "failed to persist parallel step failure"
                        );
                    }

                    let err_duration = partial.as_ref().and_then(|p| p.duration_ms).unwrap_or(0);
                    let err_cost = partial
                        .as_ref()
                        .and_then(|p| p.cost_usd)
                        .unwrap_or(Decimal::ZERO);
                    self.step_results.push(StepResult::from_failure(
                        *step_trace,
                        step_name,
                        &err_msg,
                        err_duration,
                        err_cost,
                    ));

                    if step_config.allow_failure() {
                        self.has_allowed_failure = true;
                        info!(
                            run_id = %self.run_id,
                            step = %step_name,
                            error = %err_msg,
                            "parallel step failed but allow_failure is set, continuing"
                        );
                        indexed_results[idx] = Some(Ok(allowed_failure_output(
                            &err_msg,
                            raw_response_output,
                            partial.as_ref(),
                        )));
                    } else {
                        indexed_results[idx] = Some(Err(err_msg.clone()));

                        if first_error.is_none() {
                            first_error = Some(err);
                        }

                        if fail_fast {
                            join_set.abort_all();
                        }
                    }
                }
            }
        }

        if let Some(err) = first_error {
            return Err(err);
        }

        self.persist_progress().await;

        self.last_step_ids = step_records.iter().map(|(id, _, _, _)| *id).collect();

        // Build results in original order.
        let results: Vec<ParallelStepResult> = step_records
            .iter()
            .enumerate()
            .map(|(idx, (step_id, _trace_id, name, config))| {
                let mut output = match indexed_results[idx].take() {
                    Some(Ok(o)) => o,
                    _ => unreachable!("all steps succeeded if no error returned"),
                };
                output.artifacts =
                    StepArtifacts::new(name, Some(*step_id), config.declared_outputs());
                ParallelStepResult {
                    name: name.clone(),
                    output,
                    step_id: *step_id,
                }
            })
            .collect();

        Ok(results)
    }
}
