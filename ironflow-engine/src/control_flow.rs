//! Control-flow step implementations for [`WorkflowContext`].
//!
//! Adds the [`delay`](WorkflowContext::delay) method for persistent
//! timed pauses that survive server restarts.

use chrono::{Duration, Utc};
use serde_json::json;
use tracing::info;

use ironflow_store::models::{NewStep, StepKind, StepStatus, StepUpdate, step_trace_id};

use crate::config::delay::DelayConfig;
use crate::context::WorkflowContext;
use crate::error::EngineError;

impl WorkflowContext {
    /// Execute a delay (timed pause) step.
    ///
    /// A zero-duration delay completes immediately. Otherwise, the
    /// delay step is marked completed and the method returns
    /// [`EngineError::DelaySleeping`] so the engine transitions the
    /// run to [`Sleeping`](ironflow_store::entities::RunStatus::Sleeping).
    ///
    /// On resume (after the worker picks up the re-queued run), the
    /// delay step is replayed as completed via the replay mechanism.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::DelaySleeping`] to suspend the run.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::context::WorkflowContext;
    /// use ironflow_engine::config::delay::DelayConfig;
    /// use ironflow_engine::error::EngineError;
    ///
    /// # async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    /// ctx.delay("cooldown", DelayConfig::from_secs(300)).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn delay(&mut self, name: &str, config: DelayConfig) -> Result<(), EngineError> {
        let position = self.next_position();

        if let Some(existing) = self.replay_steps().get(&position)
            && existing.kind == StepKind::Custom("delay".to_string())
            && existing.status.state == StepStatus::Completed
        {
            self.set_last_step_ids(vec![existing.id]);
            info!(
                run_id = %self.run_id(),
                step = %name,
                position,
                "delay step replayed (already completed)"
            );
            return Ok(());
        }

        let trace_id = step_trace_id(self.run_id(), name, position);
        let step = self
            .store()
            .create_step(NewStep {
                run_id: self.run_id(),
                trace_id,
                name: name.to_string(),
                kind: StepKind::Custom("delay".to_string()),
                position,
                input: Some(serde_json::to_value(&config)?),
                is_error_handler: false,
            })
            .await?;

        let now = Utc::now();
        self.start_step(step.id, now).await?;

        if config.is_zero() {
            self.store()
                .update_step(
                    step.id,
                    StepUpdate {
                        status: Some(StepStatus::Completed),
                        completed_at: Some(now),
                        ..StepUpdate::default()
                    },
                )
                .await?;
            self.set_last_step_ids(vec![step.id]);
            info!(run_id = %self.run_id(), step = %name, "delay(0) completed immediately");
            return Ok(());
        }

        let wake_at = now + Duration::seconds(config.duration_secs() as i64);

        self.store()
            .update_step(
                step.id,
                StepUpdate {
                    status: Some(StepStatus::Completed),
                    output: Some(json!({"wake_at": wake_at.to_rfc3339()})),
                    completed_at: Some(Utc::now()),
                    ..StepUpdate::default()
                },
            )
            .await?;

        self.set_last_step_ids(vec![step.id]);

        Err(EngineError::DelaySleeping {
            run_id: self.run_id(),
            step_id: step.id,
            wake_at,
        })
    }
}
