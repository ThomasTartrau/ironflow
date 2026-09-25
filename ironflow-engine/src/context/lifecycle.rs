//! Generic step lifecycle for [`WorkflowContext`].
//!
//! `execute_step` is the single path every typed step method funnels into:
//! replay, budget check, step record creation, execution (with retries),
//! persistence, events and error handlers.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde_json::{json, to_value};
use tokio::time::sleep;
use tracing::{Span, error, info, warn};
use uuid::Uuid;

use ironflow_store::models::{
    NewStep, NewStepDependency, RunUpdate, StepKind, StepStatus, StepUpdate, step_trace_id,
};

use crate::budget::step_budget_usd;
use crate::config::StepConfig;
use crate::error::EngineError;
use crate::executor::{StepArtifacts, StepOutput, StepResult, execute_step_config_intercepted};
use crate::log_sender::StepLogSender;
use crate::notify::{
    WorkflowAgentStepTokensUsedEvent, WorkflowEvent, WorkflowStepCompletedEvent,
    WorkflowStepFailedEvent, WorkflowStepStartedEvent,
};
use crate::plan::{lock_plan, planned_output};

use super::WorkflowContext;
use super::failure::{
    allowed_failure_output, extract_debug_messages_from_error, extract_partial_usage_from_error,
    extract_raw_response_from_error, is_step_retryable, record_retry_metric,
};

impl WorkflowContext {
    /// Load existing steps from the store for replay after approval.
    ///
    /// Called by the engine when resuming a run. All completed steps
    /// and the approved approval step are indexed by position so that
    /// `execute_step` and `approval` can skip them.
    ///
    /// Only steps of the current attempt are replayed: positions repeat across
    /// attempts, so replaying an earlier attempt's steps would skip the whole
    /// workflow. The one exception is an approval already granted in an earlier
    /// attempt -- approval is carried by the run, not by the attempt, so a human
    /// is never asked to approve the same gate twice.
    pub(crate) async fn load_replay_steps(&mut self) -> Result<(), EngineError> {
        let steps = self.store.list_steps(self.run_id).await?;
        for step in steps {
            let dominated = matches!(
                step.status.state,
                StepStatus::Completed | StepStatus::Running | StepStatus::AwaitingApproval
            );
            if !dominated {
                continue;
            }

            if step.attempt == self.attempt {
                self.replay_steps.insert(step.position, step);
            } else if step.kind == StepKind::Approval && step.status.state == StepStatus::Completed
            {
                self.granted_approvals.insert(step.position, step.attempt);
            }
        }
        Ok(())
    }

    /// Persist a partial snapshot of the run after a step transition.
    ///
    /// Updates the run record with the cumulative cost and duration so far,
    /// making intermediate state available for debug and recovery without
    /// waiting for `finalize_run`.
    pub(super) async fn persist_progress(&self) {
        if let Err(err) = self
            .store
            .update_run(
                self.run_id,
                RunUpdate {
                    cost_usd: Some(self.total_cost_usd),
                    duration_ms: Some(self.total_duration_ms),
                    ..RunUpdate::default()
                },
            )
            .await
        {
            warn!(
                run_id = %self.run_id,
                error = %err,
                "failed to persist run progress snapshot"
            );
        }
    }

    /// Try to replay a completed step from a previous execution.
    ///
    /// Returns `Some(StepOutput)` if a completed step exists at the given
    /// position, `None` otherwise.
    fn try_replay_step(&mut self, position: u32) -> Option<StepOutput> {
        let step = self.replay_steps.get(&position)?;
        if step.status.state != StepStatus::Completed {
            return None;
        }
        let output = StepOutput::from(step);
        self.total_cost_usd += output.cost_usd;
        self.total_duration_ms += output.duration_ms;
        self.last_step_ids = vec![step.id];
        info!(
            run_id = %self.run_id,
            step = %step.name,
            position,
            "step replayed from previous execution"
        );
        Some(output)
    }

    /// Internal: execute a step with full persistence lifecycle, and give its
    /// output the artifact handles the step can hand out.
    pub(crate) async fn execute_step(
        &mut self,
        name: &str,
        kind: StepKind,
        config: StepConfig,
    ) -> Result<StepOutput, EngineError> {
        let declared = config.declared_outputs().to_vec();
        let planning = self.is_planning();

        let mut output = self.run_step(name, kind, config).await?;

        // Every successful path records the step in `last_step_ids`; while
        // planning nothing is recorded.
        let step_id = if planning {
            None
        } else {
            self.last_step_ids.last().copied()
        };
        output.artifacts = StepArtifacts::new(name, step_id, &declared);
        Ok(output)
    }

    /// Internal: execute a step with full persistence lifecycle.
    #[tracing::instrument(
        name = "context.execute_step",
        skip_all,
        fields(
            run_id = %self.run_id,
            step.name = %name,
            step.kind,
            step.position = self.position,
            step.trace_id,
        )
    )]
    async fn run_step(
        &mut self,
        name: &str,
        kind: StepKind,
        config: StepConfig,
    ) -> Result<StepOutput, EngineError> {
        let kind_str: &'static str = match kind {
            StepKind::Shell => "shell",
            StepKind::Http => "http",
            StepKind::Agent => "agent",
            StepKind::Workflow => "workflow",
            StepKind::Approval => "approval",
            StepKind::Decision => "decision",
            StepKind::Custom(_) => "custom",
        };
        Span::current().record("step.kind", kind_str);

        // Plan mode: record the step and return a synthetic output. Nothing is
        // persisted and nothing is executed, so this must come before every
        // guard, replay and budget check below.
        if let Some(plan) = self.plan().cloned() {
            self.position += 1;
            let estimate = {
                let mut recorder = lock_plan(&plan);
                if !recorder.record(name, kind.clone(), &self.workflow_name, None) {
                    return Err(EngineError::InvalidWorkflow(
                        "execution plan exceeded the maximum number of steps".to_string(),
                    ));
                }
                recorder.set_last(vec![name.to_string()]);
                recorder.estimate_for(name)
            };
            return Ok(planned_output(&config, estimate));
        }

        // Guard timeout: checked before every step, not just sub-workflows.
        self.check_guard_timeout()?;

        let position = self.position;
        self.position += 1;

        // Replay: if this step already completed in a prior execution, return cached output.
        if let Some(output) = self.try_replay_step(position) {
            return Ok(output);
        }

        // Cost cap: refuse before creating the step record, so a run that hits
        // its cap never launches the work it cannot afford.
        if let StepConfig::Agent(ref agent_config) = config {
            self.check_run_budget(step_budget_usd(agent_config.max_budget_usd))?;
        }

        // Create step record in Pending.
        let trace_id = step_trace_id(self.run_id, name, position);
        Span::current().record("step.trace_id", trace_id.to_string().as_str());
        let step = self
            .store
            .create_step(NewStep {
                run_id: self.run_id,
                trace_id,
                name: name.to_string(),
                kind,
                position,
                input: Some(to_value(&config)?),
                is_error_handler: false,
            })
            .await?;

        self.start_step(step.id, Utc::now()).await?;

        if let Some(ref bus) = self.event_bus {
            bus.publish(
                self.run_id,
                WorkflowEvent::StepStarted(WorkflowStepStartedEvent {
                    step_name: name.to_string(),
                    step_index: position,
                    timestamp: Utc::now(),
                }),
            );
        }

        // Inputs must exist before the command runs. A failure here fails the
        // step: the command would otherwise run against missing files.
        if let Err(err) = self.prepare_step_inputs(&config, position).await {
            self.fail_step(step.id, &err).await;
            if config.allow_failure() {
                self.has_allowed_failure = true;
                self.last_step_ids = vec![step.id];
                info!(
                    run_id = %self.run_id,
                    step = %name,
                    error = %err,
                    "step input preparation failed but allow_failure is set, continuing"
                );
                return Ok(StepOutput {
                    output: json!({"error": err.to_string()}),
                    duration_ms: 0,
                    cost_usd: Decimal::ZERO,
                    input_tokens: None,
                    output_tokens: None,
                    model: None,
                    debug_messages: None,
                    artifacts: StepArtifacts::default(),
                });
            }
            return Err(err);
        }

        let mut config = config;
        let step_trace = self.trace_context.child();
        match config {
            StepConfig::Agent(ref mut agent_config) => {
                agent_config.trace_context = Some(step_trace);
            }
            StepConfig::Http(ref mut http_config) => {
                http_config.trace_context = Some(step_trace);
            }
            _ => {}
        }

        let step_log_sender = self
            .log_sender
            .as_ref()
            .map(|s| StepLogSender::new(s.clone(), self.run_id, step.id, name.to_string()));

        let execution = self
            .execute_with_guard_timeout(&config, step_log_sender)
            .await;

        let execution = self
            .retry_step_if_configured(name, kind_str, &config, step.id, execution)
            .await;

        if let Err(err) = self
            .store_step_outputs(&config, step.id, name, execution.is_ok())
            .await
        {
            self.fail_step(step.id, &err).await;
            return Err(err);
        }

        match execution {
            Ok(output) => {
                self.total_cost_usd += output.cost_usd;
                self.total_duration_ms += output.duration_ms;

                // Record token usage in the guard for agent steps.
                if matches!(config, StepConfig::Agent(_)) {
                    let tokens = output
                        .input_tokens
                        .unwrap_or(0)
                        .saturating_add(output.output_tokens.unwrap_or(0));
                    if tokens > 0 {
                        self.guard_record_tokens(tokens)?;
                    }
                }

                let debug_messages_json = output.debug_messages_json();

                let completed_at = Utc::now();
                self.store
                    .update_step(
                        step.id,
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

                self.step_results
                    .push(StepResult::from_success(trace_id, name, &output));
                self.persist_progress().await;

                info!(
                    run_id = %self.run_id,
                    step = %name,
                    trace_id = %trace_id,
                    duration_ms = output.duration_ms,
                    "step completed"
                );

                if let Some(ref bus) = self.event_bus {
                    bus.publish(
                        self.run_id,
                        WorkflowEvent::StepCompleted(WorkflowStepCompletedEvent {
                            step_name: name.to_string(),
                            step_index: position,
                            duration_ms: output.duration_ms,
                            output_summary: None,
                        }),
                    );

                    if matches!(config, StepConfig::Agent(_)) {
                        let tokens = output
                            .input_tokens
                            .unwrap_or(0)
                            .saturating_add(output.output_tokens.unwrap_or(0));
                        bus.publish(
                            self.run_id,
                            WorkflowEvent::AgentStepTokensUsed(WorkflowAgentStepTokensUsedEvent {
                                step_name: name.to_string(),
                                tokens,
                                cost_usd: output.cost_usd,
                            }),
                        );
                    }
                }

                self.last_step_ids = vec![step.id];

                Ok(output)
            }
            Err(err) => {
                let completed_at = Utc::now();
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
                        step.id,
                        StepUpdate {
                            status: Some(StepStatus::Failed),
                            error: Some(err.to_string()),
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
                    error!(step_id = %step.id, error = %store_err, "failed to persist step failure");
                }

                let err_duration = partial.as_ref().and_then(|p| p.duration_ms).unwrap_or(0);
                let err_cost = partial
                    .as_ref()
                    .and_then(|p| p.cost_usd)
                    .unwrap_or(Decimal::ZERO);
                self.step_results.push(StepResult::from_failure(
                    trace_id,
                    name,
                    &err.to_string(),
                    err_duration,
                    err_cost,
                ));
                self.persist_progress().await;

                if let Some(ref bus) = self.event_bus {
                    bus.publish(
                        self.run_id,
                        WorkflowEvent::StepFailed(WorkflowStepFailedEvent {
                            step_name: name.to_string(),
                            step_index: position,
                            error: err.to_string(),
                            duration_ms: err_duration,
                        }),
                    );
                }

                self.fire_error_handlers(name, &err.to_string(), err_duration)
                    .await;

                if config.allow_failure() {
                    self.has_allowed_failure = true;
                    self.last_step_ids = vec![step.id];
                    info!(
                        run_id = %self.run_id,
                        step = %name,
                        error = %err,
                        "step failed but allow_failure is set, continuing"
                    );
                    Ok(allowed_failure_output(
                        &err.to_string(),
                        raw_response_output,
                        partial.as_ref(),
                    ))
                } else {
                    Err(err)
                }
            }
        }
    }

    /// Retry a failed step execution when a step-level retry policy is configured
    /// and the error is transient.
    ///
    /// Returns the original result unchanged when no retry policy is set, the
    /// first attempt succeeded, or the error is not retryable.
    #[cfg_attr(not(feature = "prometheus"), allow(unused_variables))]
    async fn retry_step_if_configured(
        &self,
        name: &str,
        kind_str: &str,
        config: &StepConfig,
        step_id: Uuid,
        first_result: Result<StepOutput, EngineError>,
    ) -> Result<StepOutput, EngineError> {
        let policy = match config.retry() {
            Some(p) => p,
            None => return first_result,
        };

        let mut last_result = match first_result {
            Ok(output) => return Ok(output),
            Err(err) if !is_step_retryable(&err) => return Err(err),
            Err(err) => Err(err),
        };

        let step_log_sender = self
            .log_sender
            .as_ref()
            .map(|s| StepLogSender::new(s.clone(), self.run_id, step_id, name.to_string()));

        for attempt in 0..policy.max_retries() {
            if let StepConfig::Agent(agent_config) = config {
                self.check_run_budget(step_budget_usd(agent_config.max_budget_usd))?;
            }

            let delay = policy.delay_for_attempt(attempt);
            info!(
                run_id = %self.run_id,
                step = %name,
                attempt = attempt + 1,
                max_retries = policy.max_retries(),
                delay_ms = delay.as_millis() as u64,
                "retrying step after transient failure"
            );
            sleep(delay).await;

            record_retry_metric(kind_str, "retry");

            match execute_step_config_intercepted(
                config,
                &self.provider,
                step_log_sender.clone(),
                self.step_interceptor(),
            )
            .await
            {
                Ok(output) => return Ok(output),
                Err(err) if !is_step_retryable(&err) => return Err(err),
                err => last_result = err,
            }
        }

        record_retry_metric(kind_str, "exhausted");

        info!(
            run_id = %self.run_id,
            step = %name,
            max_retries = policy.max_retries(),
            "step retries exhausted"
        );

        last_result
    }

    /// Record dependency edges and transition a step to Running.
    ///
    /// Records edges from `step_id` to all `last_step_ids`, then
    /// transitions the step to `Running` with the given timestamp.
    pub(crate) async fn start_step(
        &self,
        step_id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<(), EngineError> {
        if !self.last_step_ids.is_empty() {
            let deps: Vec<NewStepDependency> = self
                .last_step_ids
                .iter()
                .map(|&depends_on| NewStepDependency {
                    step_id,
                    depends_on,
                })
                .collect();
            self.store.create_step_dependencies(deps).await?;
        }

        self.store
            .update_step(
                step_id,
                StepUpdate {
                    status: Some(StepStatus::Running),
                    started_at: Some(now),
                    ..StepUpdate::default()
                },
            )
            .await?;

        Ok(())
    }

    /// Mark a step as failed, best-effort.
    ///
    /// Used on paths that fail around the operation itself (artifact inputs and
    /// outputs), where the step record is already `Running` and the caller is
    /// about to propagate `err`. A store failure here is logged, never returned:
    /// it must not replace the error the caller is reporting.
    pub(super) async fn fail_step(&self, step_id: Uuid, err: &EngineError) {
        if let Err(store_err) = self
            .store
            .update_step(
                step_id,
                StepUpdate {
                    status: Some(StepStatus::Failed),
                    error: Some(err.to_string()),
                    completed_at: Some(Utc::now()),
                    ..StepUpdate::default()
                },
            )
            .await
        {
            error!(
                step_id = %step_id,
                error = %store_err,
                "failed to persist step failure"
            );
        }
    }
}
