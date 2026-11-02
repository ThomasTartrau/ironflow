//! Workflow guard enforcement for [`WorkflowContext`].
//!
//! The guard caps depth, invocation count, token usage and time across a
//! whole run tree. Tokens and active execution time add up over every
//! execution of the run (resume, requeue, retry), and the optional run
//! deadline is absolute, counted from the creation of the run. These helpers
//! read the shared state before and after every step so the limits apply
//! globally, not per workflow.

use std::time::Duration;

use tokio::time::timeout;
use tracing::error;

use crate::config::StepConfig;
use crate::error::EngineError;
use crate::executor::{StepOutput, execute_step_config_intercepted};
use crate::guard::WorkflowRejection;
use crate::log_sender::StepLogSender;

use super::WorkflowContext;

impl WorkflowContext {
    /// Decrement guard state after a sub-workflow returns (success or failure).
    ///
    /// Logs on poison rather than propagating, because this runs on error
    /// paths where the workflow is already failing.
    pub(super) fn guard_record_return(&self) {
        if let Some(guard_state) = &self.guard_state {
            match guard_state.lock() {
                Ok(mut state) => state.record_return(),
                Err(_) => {
                    error!(
                        run_id = %self.run_id,
                        "guard state mutex poisoned in record_return"
                    );
                }
            }
        }
    }

    /// Wrap step execution with the time left before the nearest guard time
    /// limit (timeout or run deadline).
    ///
    /// When no guard is configured the step runs without a timeout wrapper.
    pub(super) async fn execute_with_guard_timeout(
        &self,
        config: &StepConfig,
        step_log_sender: Option<StepLogSender>,
    ) -> Result<StepOutput, EngineError> {
        match self.guard_time_limit() {
            Some((dur, rejection)) => {
                match timeout(
                    dur,
                    execute_step_config_intercepted(
                        config,
                        &self.provider,
                        step_log_sender,
                        self.step_interceptor(),
                    ),
                )
                .await
                {
                    Ok(result) => result,
                    Err(_elapsed) => Err(rejection.into()),
                }
            }
            None => {
                execute_step_config_intercepted(
                    config,
                    &self.provider,
                    step_log_sender,
                    self.step_interceptor(),
                )
                .await
            }
        }
    }

    /// The time left before the nearest guard time limit, and the rejection
    /// to raise when it runs out. `None` without a guard.
    pub(super) fn guard_time_limit(&self) -> Option<(Duration, WorkflowRejection)> {
        let config = self.guard_config.as_ref()?;
        let guard_state = self.guard_state.as_ref()?;
        let state = guard_state.lock().ok()?;
        Some(state.time_limit(config))
    }

    /// Check the guard timeout and the run deadline before every step.
    pub(super) fn check_guard_timeout(&self) -> Result<(), EngineError> {
        if let (Some(config), Some(guard_state)) = (&self.guard_config, &self.guard_state) {
            let state = guard_state
                .lock()
                .map_err(|_| WorkflowRejection::GuardUnavailable)?;
            state.check_time(config)?;
        }
        Ok(())
    }

    /// Record token usage from an agent step in the guard state.
    pub(super) fn guard_record_tokens(&self, tokens: u64) -> Result<(), EngineError> {
        if let (Some(config), Some(guard_state)) = (&self.guard_config, &self.guard_state) {
            let mut state = guard_state
                .lock()
                .map_err(|_| WorkflowRejection::GuardUnavailable)?;
            state.record_tokens(config, tokens)?;
        }
        Ok(())
    }
}
