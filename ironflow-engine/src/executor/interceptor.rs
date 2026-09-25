//! Hook that resolves a step without executing it.
//!
//! A [`StepInterceptor`] is consulted by
//! [`execute_step_config_intercepted`](crate::executor::execute_step_config_intercepted)
//! before the dispatcher picks an executor, and by
//! [`WorkflowContext::approval`](crate::context::WorkflowContext::approval)
//! before the gate suspends the run. Returning `Some(..)` short-circuits the
//! step: no process is spawned, no request is sent, no human is asked.
//!
//! Production wiring leaves the hook unset. In practice the only implementor is
//! [`crate::testing`], which uses it to run a handler's real logic against
//! canned step results.

use crate::config::{ApprovalConfig, StepConfig};
use crate::error::EngineError;
use crate::executor::StepOutput;

/// Decision applied to an approval gate by a [`StepInterceptor`].
///
/// # Examples
///
/// ```
/// use ironflow_engine::executor::ApprovalOutcome;
///
/// let granted = ApprovalOutcome::Approved;
/// assert_eq!(granted, ApprovalOutcome::Approved);
///
/// let refused = ApprovalOutcome::Rejected { reason: "not on a Friday".to_string() };
/// assert_ne!(granted, refused);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalOutcome {
    /// The gate is granted; execution continues past it.
    Approved,
    /// The gate is refused; the run fails with [`EngineError::ApprovalRejected`].
    Rejected {
        /// Human-readable reason recorded on the step and on the run.
        reason: String,
    },
}

impl ApprovalOutcome {
    /// Build an [`ApprovalOutcome::Rejected`] with the given reason.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::executor::ApprovalOutcome;
    ///
    /// let outcome = ApprovalOutcome::reject("budget freeze");
    /// assert_eq!(
    ///     outcome,
    ///     ApprovalOutcome::Rejected { reason: "budget freeze".to_string() }
    /// );
    /// ```
    pub fn reject(reason: &str) -> Self {
        Self::Rejected {
            reason: reason.to_string(),
        }
    }
}

/// Resolves steps without executing them.
///
/// # Examples
///
/// ```
/// use ironflow_engine::config::{ShellConfig, StepConfig};
/// use ironflow_engine::error::EngineError;
/// use ironflow_engine::executor::{StepArtifacts, StepInterceptor, StepOutput};
/// use rust_decimal::Decimal;
/// use serde_json::json;
///
/// struct AlwaysOk;
///
/// impl StepInterceptor for AlwaysOk {
///     fn intercept(&self, config: &StepConfig) -> Option<Result<StepOutput, EngineError>> {
///         match config {
///             StepConfig::Shell(_) => Some(Ok(StepOutput {
///                 output: json!({"stdout": "ok", "stderr": "", "exit_code": 0}),
///                 duration_ms: 0,
///                 cost_usd: Decimal::ZERO,
///                 input_tokens: None,
///                 output_tokens: None,
///                 model: None,
///                 debug_messages: None,
///                 artifacts: StepArtifacts::default(),
///             })),
///             _ => None,
///         }
///     }
/// }
///
/// let config = StepConfig::Shell(ShellConfig::new("./deploy.sh"));
/// let intercepted = AlwaysOk.intercept(&config).expect("shell is intercepted");
/// assert_eq!(intercepted.expect("canned output").stdout(), "ok");
/// ```
pub trait StepInterceptor: Send + Sync {
    /// Return `Some(result)` to short-circuit this step, `None` to execute it
    /// for real.
    fn intercept(&self, config: &StepConfig) -> Option<Result<StepOutput, EngineError>>;

    /// Resolve an approval gate instead of suspending the run.
    ///
    /// The default implementation returns `None`: the gate suspends the run.
    fn intercept_approval(&self, name: &str, config: &ApprovalConfig) -> Option<ApprovalOutcome> {
        let _ = (name, config);
        None
    }
}
