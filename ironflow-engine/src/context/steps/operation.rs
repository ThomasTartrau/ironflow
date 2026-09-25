//! Custom operation step for [`WorkflowContext`].

use std::time::Instant;

use chrono::Utc;
use rust_decimal::Decimal;
use tracing::{error, info};

use ironflow_store::models::{NewStep, StepKind, StepStatus, StepUpdate, step_trace_id};

use crate::context::WorkflowContext;
use crate::error::EngineError;
use crate::executor::StepOutput;
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
    /// # Errors
    ///
    /// Returns [`EngineError`] if the operation fails or the store errors.
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
        let kind = StepKind::Custom(op.kind().to_string());

        // Plan mode: `op.execute` is never called, so no third-party API is
        // touched while planning.
        if let Some(plan) = self.plan().cloned() {
            self.position += 1;
            let mut recorder = lock_plan(&plan);
            let estimate = recorder.estimate_for(name);
            if recorder.record(name, kind, &self.workflow_name, None) {
                recorder.set_last(vec![name.to_string()]);
            }
            return Ok(planned_custom_output(estimate));
        }

        let position = self.position;
        self.position += 1;

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
                self.store
                    .update_step(
                        step.id,
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

                self.last_step_ids = vec![step.id];

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

                Err(engine_err)
            }
        }
    }
}
