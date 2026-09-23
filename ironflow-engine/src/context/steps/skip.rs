//! Explicitly skipped step for [`WorkflowContext`].

use chrono::Utc;
use serde_json::json;
use tracing::info;

use ironflow_store::models::{
    NewStep, NewStepDependency, StepKind, StepStatus, StepUpdate, step_trace_id,
};

use crate::context::WorkflowContext;
use crate::error::EngineError;
use crate::plan::{ConditionResult, lock_plan};

impl WorkflowContext {
    /// Record a step as explicitly skipped.
    ///
    /// Use this inside an `if`/`else` branch when a step should not execute
    /// but must still appear in the DAG and timeline with its reason.
    ///
    /// The step is created directly in [`StepStatus::Skipped`] state and the
    /// reason is stored in the output as `{"reason": "..."}`.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] if the store fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::context::WorkflowContext;
    /// use ironflow_engine::error::EngineError;
    ///
    /// # async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    /// let tests_passed = false;
    /// if tests_passed {
    ///     // ctx.shell("deploy", ...).await?;
    /// } else {
    ///     ctx.skip("deploy", "tests failed").await?;
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn skip(&mut self, name: &str, reason: &str) -> Result<(), EngineError> {
        // Plan mode: the skip and its reason become the step's condition.
        if let Some(plan) = self.plan().cloned() {
            self.position += 1;
            let mut recorder = lock_plan(&plan);
            recorder.set_condition(ConditionResult::Skipped {
                reason: reason.to_string(),
            });
            if recorder.record(
                name,
                StepKind::Custom("skip".to_string()),
                &self.workflow_name,
                None,
            ) {
                recorder.set_last(vec![name.to_string()]);
            }
            return Ok(());
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
                kind: StepKind::Custom("skip".to_string()),
                position,
                input: None,
                is_error_handler: false,
            })
            .await?;

        if !self.last_step_ids.is_empty() {
            let deps: Vec<NewStepDependency> = self
                .last_step_ids
                .iter()
                .map(|&depends_on| NewStepDependency {
                    step_id: step.id,
                    depends_on,
                })
                .collect();
            self.store.create_step_dependencies(deps).await?;
        }

        let now = Utc::now();
        self.store
            .update_step(
                step.id,
                StepUpdate {
                    status: Some(StepStatus::Skipped),
                    output: Some(json!({"reason": reason})),
                    completed_at: Some(now),
                    ..StepUpdate::default()
                },
            )
            .await?;

        self.last_step_ids = vec![step.id];

        info!(
            run_id = %self.run_id,
            step = %name,
            reason,
            "step skipped"
        );

        Ok(())
    }
}
