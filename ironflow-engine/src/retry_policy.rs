//! Automatic retry policy for failed runs.
//!
//! When a run fails and still has retries left, the engine reschedules it
//! instead of marking it `Failed`. This module owns the two decisions involved:
//!
//! * [`is_run_retryable`] -- is this failure worth replaying at all?
//! * [`backoff_for_retry`] -- how long to wait before the next attempt?
//!
//! This is **run-level** retry: the whole run is replayed from the start.
//! It is distinct from [`ironflow_core::retry`], which retries a single
//! operation (an HTTP call, an agent invocation) in place, with much shorter
//! delays.
//!
//! # Examples
//!
//! ```
//! use std::time::Duration;
//! use ironflow_engine::error::EngineError;
//! use ironflow_engine::retry_policy::{backoff_for_retry, is_run_retryable};
//!
//! // A missing handler will never succeed on replay.
//! let err = EngineError::InvalidWorkflow("no handler registered: deploy".to_string());
//! assert!(!is_run_retryable(&err));
//!
//! // The first retry waits roughly 30 seconds.
//! let delay = backoff_for_retry(0);
//! assert!(delay >= Duration::from_secs(24) && delay <= Duration::from_secs(36));
//! ```

use std::time::Duration;

use ironflow_core::retry::is_retryable as is_operation_retryable;
use rand::Rng;

use crate::error::EngineError;

/// Delay before the first retry.
const INITIAL_BACKOFF: Duration = Duration::from_secs(30);

/// Growth factor applied to the backoff after each attempt.
///
/// With a 30 s base this yields 30 s, 2 min, 8 min, then the cap.
const BACKOFF_MULTIPLIER: f64 = 4.0;

/// Upper bound on the backoff, however many retries have been consumed.
const MAX_BACKOFF: Duration = Duration::from_secs(15 * 60);

/// Relative jitter applied to the computed backoff (+/- 20%).
///
/// Spreads out retries so that runs failing together on the same transient
/// outage do not all come back at the same instant.
const JITTER_RATIO: f64 = 0.2;

/// Returns `true` if a failed run is worth replaying from the start.
///
/// # Non-retryable failures
///
/// These consume no attempt, because replaying them cannot change the outcome:
///
/// | Error | Reason |
/// |-------|--------|
/// | [`EngineError::InvalidWorkflow`] | the handler is not registered |
/// | [`EngineError::Serialization`] | the payload or step config is malformed |
/// | [`EngineError::StepConfig`] | the step config cannot be deserialized |
/// | [`EngineError::Store`] | if the store is down, persisting the retry fails too |
/// | [`EngineError::ApprovalRequired`] | not a failure; the run is suspended, not failed |
///
/// [`EngineError::Operation`] delegates to
/// [`ironflow_core::retry::is_retryable`], so a 5xx or an agent timeout is
/// replayed while a shell non-zero exit or an exhausted budget is not.
///
/// Approval rejections and manual cancellations never reach this function:
/// they transition the run directly through the API and so never consume an
/// attempt either.
///
/// # Examples
///
/// ```
/// use ironflow_engine::error::EngineError;
/// use ironflow_engine::retry_policy::is_run_retryable;
///
/// assert!(!is_run_retryable(&EngineError::StepConfig("bad shell config".to_string())));
/// ```
pub fn is_run_retryable(error: &EngineError) -> bool {
    match error {
        EngineError::Operation(op_err) => is_operation_retryable(op_err),
        EngineError::InvalidWorkflow(_)
        | EngineError::StepConfig(_)
        | EngineError::Serialization(_)
        | EngineError::Store(_)
        | EngineError::ApprovalRequired { .. } => false,
    }
}

/// Compute how long to wait before the given retry (0-indexed).
///
/// `retry_index` is the number of retries already consumed, so `0` is the delay
/// before the first replay. The result grows exponentially, is capped at 15
/// minutes, and carries +/- 20% jitter.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
/// use ironflow_engine::retry_policy::backoff_for_retry;
///
/// // Capped at 15 minutes, plus at most 20% of jitter, however far the run
/// // has gone.
/// let delay = backoff_for_retry(50);
/// assert!(delay <= Duration::from_secs(18 * 60));
/// ```
pub fn backoff_for_retry(retry_index: u32) -> Duration {
    let jitter = rand::rng().random_range(1.0 - JITTER_RATIO..=1.0 + JITTER_RATIO);
    jittered_backoff(retry_index, jitter)
}

/// Backoff computation with the jitter factor supplied by the caller.
///
/// Split out from [`backoff_for_retry`] so the growth curve and the cap can be
/// asserted deterministically.
fn jittered_backoff(retry_index: u32, jitter: f64) -> Duration {
    let base = INITIAL_BACKOFF.as_secs_f64() * BACKOFF_MULTIPLIER.powi(retry_index as i32);
    let capped = base.min(MAX_BACKOFF.as_secs_f64());
    Duration::from_secs_f64(capped * jitter)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use ironflow_core::error::{AgentError, OperationError};
    use ironflow_store::error::StoreError;
    use uuid::Uuid;

    use super::*;

    // --- is_run_retryable ---

    #[test]
    fn invalid_workflow_is_not_retryable() {
        let err = EngineError::InvalidWorkflow("no handler registered: deploy".to_string());
        assert!(!is_run_retryable(&err));
    }

    #[test]
    fn step_config_is_not_retryable() {
        let err = EngineError::StepConfig("bad shell config".to_string());
        assert!(!is_run_retryable(&err));
    }

    #[test]
    fn serialization_is_not_retryable() {
        let serde_err = serde_json::from_str::<String>("not json").unwrap_err();
        assert!(!is_run_retryable(&EngineError::Serialization(serde_err)));
    }

    #[test]
    fn store_error_is_not_retryable() {
        let err = EngineError::Store(StoreError::RunNotFound(Uuid::nil()));
        assert!(!is_run_retryable(&err));
    }

    #[test]
    fn approval_required_is_not_retryable() {
        let err = EngineError::ApprovalRequired {
            run_id: Uuid::nil(),
            step_id: Uuid::nil(),
            message: "approve?".to_string(),
        };
        assert!(!is_run_retryable(&err));
    }

    #[test]
    fn budget_exceeded_is_not_retryable() {
        let err = EngineError::Operation(OperationError::Agent(AgentError::BudgetExceeded {
            spent_usd: 0.30,
            limit_usd: 0.25,
            debug_messages: Vec::new(),
            partial_usage: Box::default(),
        }));
        assert!(!is_run_retryable(&err));
    }

    #[test]
    fn shell_failure_is_not_retryable() {
        let err = EngineError::Operation(OperationError::Shell {
            exit_code: 1,
            stderr: "boom".to_string(),
        });
        assert!(!is_run_retryable(&err));
    }

    #[test]
    fn http_server_error_is_retryable() {
        let err = EngineError::Operation(OperationError::Http {
            status: Some(503),
            message: "service unavailable".to_string(),
        });
        assert!(is_run_retryable(&err));
    }

    #[test]
    fn http_client_error_is_not_retryable() {
        let err = EngineError::Operation(OperationError::Http {
            status: Some(422),
            message: "unprocessable".to_string(),
        });
        assert!(!is_run_retryable(&err));
    }

    #[test]
    fn operation_timeout_is_retryable() {
        let err = EngineError::Operation(OperationError::Timeout {
            step: "build".to_string(),
            limit: Duration::from_secs(60),
        });
        assert!(is_run_retryable(&err));
    }

    #[test]
    fn agent_process_failure_is_retryable() {
        let err = EngineError::Operation(OperationError::Agent(AgentError::ProcessFailed {
            exit_code: 1,
            stderr: "crashed".to_string(),
        }));
        assert!(is_run_retryable(&err));
    }

    // --- backoff ---

    #[test]
    fn backoff_grows_by_the_multiplier() {
        assert_eq!(jittered_backoff(0, 1.0), Duration::from_secs(30));
        assert_eq!(jittered_backoff(1, 1.0), Duration::from_secs(120));
        assert_eq!(jittered_backoff(2, 1.0), Duration::from_secs(480));
    }

    #[test]
    fn backoff_is_capped() {
        assert_eq!(jittered_backoff(3, 1.0), Duration::from_secs(15 * 60));
        assert_eq!(jittered_backoff(50, 1.0), Duration::from_secs(15 * 60));
    }

    #[test]
    fn jitter_scales_the_delay() {
        assert_eq!(jittered_backoff(0, 0.8), Duration::from_secs(24));
        assert_eq!(jittered_backoff(0, 1.2), Duration::from_secs(36));
    }

    #[test]
    fn backoff_for_retry_stays_within_jitter_bounds() {
        for _ in 0..100 {
            let delay = backoff_for_retry(0);
            assert!(
                delay >= Duration::from_secs(24) && delay <= Duration::from_secs(36),
                "delay {delay:?} outside +/- 20% of 30s"
            );
        }
    }

    #[test]
    fn backoff_for_retry_never_exceeds_the_cap_with_jitter() {
        for _ in 0..100 {
            let delay = backoff_for_retry(10);
            assert!(delay <= Duration::from_secs_f64(15.0 * 60.0 * 1.2));
        }
    }
}
