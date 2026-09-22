//! Sub-workflow step for [`WorkflowContext`].
//!
//! A sub-workflow runs a registered [`WorkflowHandler`] in its own child run.
//! The child context is built here from the parent's private fields, which is
//! possible because this module is a descendant of `context`.

use std::collections::HashMap;
use std::time::Instant;

use chrono::Utc;
use rust_decimal::Decimal;
use serde_json::{Value, json, to_value};
use tracing::{error, info};

use ironflow_store::models::{
    NewRun, NewStep, RunStatus, RunUpdate, StepKind, StepStatus, StepUpdate, TriggerKind,
    step_trace_id,
};

use crate::config::WorkflowStepConfig;
use crate::context::WorkflowContext;
use crate::error::EngineError;
use crate::executor::StepOutput;
use crate::guard::WorkflowRejection;
use crate::handler::WorkflowHandler;

impl WorkflowContext {
    /// Execute a sub-workflow step.
    ///
    /// Creates a child run for the named workflow handler, executes it with
    /// its own steps and lifecycle, and returns a [`StepOutput`] containing
    /// the child run ID and aggregated metrics.
    ///
    /// Requires the context to be created with
    /// `with_handler_resolver`.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::InvalidWorkflow`] if no handler is registered
    /// with the given name, or if no handler resolver is available.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::context::WorkflowContext;
    /// use ironflow_engine::error::EngineError;
    /// use serde_json::json;
    ///
    /// # async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    /// // let result = ctx.workflow(&MySubWorkflow, json!({})).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn workflow(
        &mut self,
        handler: &dyn WorkflowHandler,
        payload: Value,
    ) -> Result<StepOutput, EngineError> {
        // Guard check: verify limits before creating the step.
        if let (Some(guard_config), Some(guard_state)) = (&self.guard_config, &self.guard_state) {
            let state = guard_state
                .lock()
                .map_err(|_| WorkflowRejection::GuardUnavailable)?;
            state.check(guard_config, handler.name())?;
        }

        let config = WorkflowStepConfig::new(handler.name(), payload);
        let position = self.position;
        self.position += 1;

        let trace_id = step_trace_id(self.run_id, &config.workflow_name, position);
        let step = self
            .store
            .create_step(NewStep {
                run_id: self.run_id,
                trace_id,
                name: config.workflow_name.clone(),
                kind: StepKind::Workflow,
                position,
                input: Some(to_value(&config)?),
                is_error_handler: false,
            })
            .await?;

        self.start_step(step.id, Utc::now()).await?;

        // Record invocation in guard state (fail-closed).
        if let Some(guard_state) = &self.guard_state {
            let mut state = guard_state
                .lock()
                .map_err(|_| WorkflowRejection::GuardUnavailable)?;
            state.record_invocation(handler.name());
        }

        match self.execute_child_workflow(&config).await {
            Ok((output, child_had_allowed_failure)) => {
                self.total_cost_usd += output.cost_usd;
                self.total_duration_ms += output.duration_ms;
                if child_had_allowed_failure {
                    self.has_allowed_failure = true;
                }

                let completed_at = Utc::now();
                self.store
                    .update_step(
                        step.id,
                        StepUpdate {
                            status: Some(StepStatus::Completed),
                            output: Some(output.output.clone()),
                            duration_ms: Some(output.duration_ms),
                            cost_usd: Some(output.cost_usd),
                            completed_at: Some(completed_at),
                            ..StepUpdate::default()
                        },
                    )
                    .await?;

                info!(
                    run_id = %self.run_id,
                    child_workflow = %config.workflow_name,
                    duration_ms = output.duration_ms,
                    "workflow step completed"
                );

                self.last_step_ids = vec![step.id];

                self.guard_record_return();
                Ok(output)
            }
            Err(err) => {
                let completed_at = Utc::now();
                if let Err(store_err) = self
                    .store
                    .update_step(
                        step.id,
                        StepUpdate {
                            status: Some(StepStatus::Failed),
                            error: Some(err.to_string()),
                            completed_at: Some(completed_at),
                            ..StepUpdate::default()
                        },
                    )
                    .await
                {
                    error!(step_id = %step.id, error = %store_err, "failed to persist step failure");
                }

                self.guard_record_return();
                Err(err)
            }
        }
    }

    /// Execute a child workflow and return aggregated output plus whether
    /// at least one `allow_failure` step failed.
    async fn execute_child_workflow(
        &self,
        config: &WorkflowStepConfig,
    ) -> Result<(StepOutput, bool), EngineError> {
        let resolver = self.handler_resolver.as_ref().ok_or_else(|| {
            EngineError::InvalidWorkflow(
                "sub-workflow requires a handler resolver (use Engine to execute)".to_string(),
            )
        })?;

        let handler = resolver(&config.workflow_name).ok_or_else(|| {
            EngineError::InvalidWorkflow(format!("no handler registered: {}", config.workflow_name))
        })?;

        // A child run inherits both the parent labels and the parent author:
        // whoever triggered the parent workflow is accountable for its children.
        let parent = self.store.get_run(self.run_id).await?;
        let (parent_labels, parent_author) =
            parent.map(|r| (r.labels, r.created_by)).unwrap_or_default();

        let child_run = self
            .store
            .create_run(NewRun {
                workflow_name: config.workflow_name.clone(),
                trigger: TriggerKind::Workflow,
                payload: config.payload.clone(),
                max_retries: 0,
                handler_version: None,
                labels: parent_labels,
                scheduled_at: None,
                created_by: parent_author,
                idempotency_key: None,
                // The child shares the parent's cap; it does not get its own budget.
                max_cost_usd: self.max_cost_usd,
            })
            .await?
            .into_run();

        let child_run_id = child_run.id;
        info!(
            parent_run_id = %self.run_id,
            child_run_id = %child_run_id,
            workflow = %config.workflow_name,
            "child run created"
        );

        self.store
            .update_run_status(child_run_id, RunStatus::Running)
            .await?;

        let run_start = Instant::now();
        let mut child_ctx = WorkflowContext {
            run_id: child_run_id,
            workflow_name: config.workflow_name.clone(),
            store: self.store.clone(),
            provider: self.provider.clone(),
            decision_provider: self.decision_provider.clone(),
            handler_resolver: self.handler_resolver.clone(),
            position: 0,
            last_step_ids: Vec::new(),
            total_cost_usd: Decimal::ZERO,
            total_duration_ms: 0,
            max_cost_usd: self.max_cost_usd,
            // Everything the parent chain already spent counts against the
            // shared cap, so the child cannot restart the budget from zero.
            inherited_cost_usd: self.charged_cost_usd(),
            replay_steps: HashMap::new(),
            granted_approvals: HashMap::new(),
            // A child run is created fresh here; it is never itself retried.
            attempt: 1,
            carried_duration_ms: 0,
            log_sender: self.log_sender.clone(),
            // A child shares the storage backend but not the parent's artifacts:
            // input lookups are scoped to the child's own run.
            artifact_sink: self.artifact_sink.clone(),
            has_allowed_failure: false,
            error_handlers: Vec::new(),
            guard_state: self.guard_state.clone(),
            guard_config: self.guard_config.clone(),
            step_results: Vec::new(),
            event_bus: self.event_bus.clone(),
            trace_context: self.trace_context.child(),
            operation_ctx: None,
        };

        let result = handler.execute(&mut child_ctx).await;
        let total_duration = run_start.elapsed().as_millis() as u64;
        let completed_at = Utc::now();

        match result {
            Ok(()) => {
                let child_status = if child_ctx.has_allowed_failure {
                    RunStatus::Warning
                } else {
                    RunStatus::Completed
                };
                self.store
                    .update_run(
                        child_run_id,
                        RunUpdate {
                            status: Some(child_status),
                            cost_usd: Some(child_ctx.total_cost_usd),
                            duration_ms: Some(total_duration),
                            completed_at: Some(completed_at),
                            ..RunUpdate::default()
                        },
                    )
                    .await?;

                let child_had_allowed_failure = child_ctx.has_allowed_failure;
                Ok((
                    StepOutput {
                        output: json!({
                            "run_id": child_run_id,
                            "workflow_name": config.workflow_name,
                            "status": child_status,
                            "cost_usd": child_ctx.total_cost_usd,
                            "duration_ms": total_duration,
                        }),
                        duration_ms: total_duration,
                        cost_usd: child_ctx.total_cost_usd,
                        input_tokens: None,
                        output_tokens: None,
                        model: None,
                        debug_messages: None,
                    },
                    child_had_allowed_failure,
                ))
            }
            Err(err) => {
                if let Err(store_err) = self
                    .store
                    .update_run(
                        child_run_id,
                        RunUpdate {
                            status: Some(RunStatus::Failed),
                            error: Some(err.to_string()),
                            cost_usd: Some(child_ctx.total_cost_usd),
                            duration_ms: Some(total_duration),
                            completed_at: Some(completed_at),
                            ..RunUpdate::default()
                        },
                    )
                    .await
                {
                    error!(
                        child_run_id = %child_run_id,
                        store_error = %store_err,
                        "failed to persist child run failure"
                    );
                }

                Err(err)
            }
        }
    }
}
