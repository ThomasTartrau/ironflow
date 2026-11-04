//! Custom operation step for [`WorkflowContext`].

use std::time::Instant;

use chrono::Utc;
use rust_decimal::Decimal;
use tracing::{error, info};

use ironflow_store::models::{NewStep, StepKind, StepStatus, StepUpdate, step_trace_id};

use crate::context::WorkflowContext;
use crate::context::failure::allowed_failure_output;
use crate::error::EngineError;
use crate::executor::{StepArtifacts, StepOutput};
use crate::operation::Operation;
use crate::plan::{lock_plan, planned_custom_output};

impl WorkflowContext {
    /// Execute a custom operation step.
    ///
    /// Runs a user-defined [`Operation`] with full step lifecycle management:
    /// creates the step record, transitions to Running, executes the operation,
    /// persists the output and duration, and marks the step Completed or Failed.
    ///
    /// The operation's [`kind()`](Operation::kind) is stored as
    /// [`StepKind::Custom`].
    ///
    /// On resume, a step that already completed in a prior execution of the
    /// current attempt is replayed from the store instead of calling
    /// [`Operation::execute`] again.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] if the operation fails or the store errors.
    /// Returns [`EngineError::ReplayDivergence`] when the step recorded at
    /// this position has a different name or kind.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use async_trait::async_trait;
    /// use ironflow_engine::context::WorkflowContext;
    /// use ironflow_engine::operation::{Operation, OperationContext};
    /// use ironflow_core::error::OperationError;
    /// use ironflow_engine::error::EngineError;
    /// use serde_json::{Value, json};
    ///
    /// struct MyOp;
    /// #[async_trait]
    /// impl Operation for MyOp {
    ///     fn kind(&self) -> &str { "my-service" }
    ///     async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
    ///         Ok(json!({"ok": true}))
    ///     }
    /// }
    ///
    /// # async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    /// let result = ctx.operation("call-service", &MyOp).await?;
    /// println!("output: {}", result.output);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn operation(
        &mut self,
        name: &str,
        op: &dyn Operation,
    ) -> Result<StepOutput, EngineError> {
        self.run_operation(name, op, false).await
    }

    /// Execute a custom operation step whose failure does not stop the run.
    ///
    /// Same as [`operation`](Self::operation), except when
    /// [`Operation::execute`] fails: the step is recorded as Failed, the run
    /// ends as `Warning` instead of `Completed`, and the handler receives an
    /// output whose [`error()`](StepOutput::error) is the failure. On resume,
    /// the tolerated failure is replayed with that output, not executed again.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] if the store errors, or
    /// [`EngineError::ReplayDivergence`] when the step recorded at this
    /// position has a different name or kind. Never for a failure of the
    /// operation itself.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::context::WorkflowContext;
    /// use ironflow_engine::error::EngineError;
    /// use ironflow_engine::operation::Operation;
    ///
    /// # async fn example(ctx: &mut WorkflowContext, op: &dyn Operation) -> Result<(), EngineError> {
    /// let result = ctx.operation_allow_failure("regenerate", op).await?;
    /// if let Some(error) = result.error() {
    ///     println!("regeneration failed, carrying on: {error}");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn operation_allow_failure(
        &mut self,
        name: &str,
        op: &dyn Operation,
    ) -> Result<StepOutput, EngineError> {
        self.run_operation(name, op, true).await
    }

    async fn run_operation(
        &mut self,
        name: &str,
        op: &dyn Operation,
        allow_failure: bool,
    ) -> Result<StepOutput, EngineError> {
        let kind = StepKind::Custom(op.kind().to_string());

        // Plan mode: `op.execute` is never called, so no third-party API is
        // touched while planning.
        if let Some(plan) = self.plan().cloned() {
            self.position += 1;
            let mut recorder = lock_plan(&plan);
            let estimate = recorder.estimate_for(name);
            recorder.record_or_stop(name, kind, &self.workflow_name, None)?;
            recorder.set_last(vec![name.to_string()]);
            let mut output = planned_custom_output(estimate);
            output.artifacts = StepArtifacts::new(name, None, &[]);
            return Ok(output);
        }

        let position = self.position;
        self.position += 1;

        // Replay: if this step already completed, or failed while tolerated,
        // in a prior execution of the current attempt, return its recorded
        // output without calling `op.execute` or creating a new step.
        if let Some(mut output) = self.try_replay_step(position, name, &kind, allow_failure)? {
            let step_id = self.last_step_ids.last().copied();
            output.artifacts = StepArtifacts::new(name, step_id, &[]);
            return Ok(output);
        }

        let trace_id = step_trace_id(self.run_id, name, position);
        let step = self
            .store
            .create_step(NewStep {
                run_id: self.run_id,
                trace_id,
                name: name.to_string(),
                kind,
                position,
                input: op.input(),
                is_error_handler: false,
            })
            .await?;

        self.start_step(step.id, Utc::now()).await?;

        let start = Instant::now();

        let op_ctx = self.ensure_operation_ctx();

        match op.execute(op_ctx).await {
            Ok(output_value) => {
                let duration_ms = start.elapsed().as_millis() as u64;
                self.total_duration_ms += duration_ms;

                let completed_at = Utc::now();
                let step_id = self
                    .complete_step(
                        &step,
                        StepUpdate {
                            status: Some(StepStatus::Completed),
                            output: Some(output_value.clone()),
                            duration_ms: Some(duration_ms),
                            cost_usd: Some(Decimal::ZERO),
                            completed_at: Some(completed_at),
                            ..StepUpdate::default()
                        },
                    )
                    .await?;

                info!(
                    run_id = %self.run_id,
                    step = %name,
                    kind = op.kind(),
                    duration_ms,
                    "operation step completed"
                );

                self.last_step_ids = vec![step_id];

                Ok(StepOutput {
                    output: output_value,
                    duration_ms,
                    cost_usd: Decimal::ZERO,
                    input_tokens: None,
                    cache_read_input_tokens: None,
                    cache_creation_input_tokens: None,
                    output_tokens: None,
                    model: None,
                    debug_messages: None,
                    // An operation declares no output; its record is what
                    // `put_artifact` attaches bytes to.
                    artifacts: StepArtifacts::new(name, Some(step_id), &[]),
                    account_id: None,
                    environment_id: None,
                })
            }
            Err(err) => {
                let completed_at = Utc::now();
                let engine_err = EngineError::Operation(err);
                if let Err(store_err) = self
                    .store
                    .update_step(
                        step.id,
                        StepUpdate {
                            status: Some(StepStatus::Failed),
                            error: Some(engine_err.to_string()),
                            completed_at: Some(completed_at),
                            ..StepUpdate::default()
                        },
                    )
                    .await
                {
                    error!(step_id = %step.id, error = %store_err, "failed to persist step failure");
                }

                if !allow_failure {
                    return Err(engine_err);
                }
                self.has_allowed_failure = true;
                self.last_step_ids = vec![step.id];
                info!(
                    run_id = %self.run_id,
                    step = %name,
                    kind = op.kind(),
                    error = %engine_err,
                    "operation step failed but allow_failure is set, continuing"
                );
                let mut output = allowed_failure_output(&engine_err.to_string(), None, None);
                output.duration_ms = start.elapsed().as_millis() as u64;
                self.total_duration_ms += output.duration_ms;
                output.artifacts = StepArtifacts::new(name, Some(step.id), &[]);
                Ok(output)
            }
        }
    }
}
