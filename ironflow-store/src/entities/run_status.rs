//! [`RunStatus`] FSM — lifecycle states for a workflow run.

use serde::{Deserialize, Serialize};

/// Status of a workflow run, forming a finite state machine.
///
/// Valid transitions:
/// - `Pending` -> `Running`, `Cancelled`
/// - `Running` -> `Completed`, `Failed`, `Retrying`, `Cancelled`, `AwaitingApproval`
/// - `Retrying` -> `Running`, `Failed`, `Cancelled`
/// - `AwaitingApproval` -> `Running`, `Failed`, `Cancelled`
///
/// Terminal states (`Completed`, `Failed`, `Cancelled`) are idempotent:
/// transitioning to the same terminal state is a no-op, not an error.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::RunStatus;
///
/// assert!(RunStatus::Pending.can_transition_to(&RunStatus::Running));
/// assert!(!RunStatus::Pending.can_transition_to(&RunStatus::Completed));
/// assert!(!RunStatus::Completed.can_transition_to(&RunStatus::Running));
/// assert!(RunStatus::Running.can_transition_to(&RunStatus::AwaitingApproval));
/// assert!(RunStatus::AwaitingApproval.can_transition_to(&RunStatus::Running));
/// // Terminal-to-same is idempotent:
/// assert!(RunStatus::Failed.can_transition_to(&RunStatus::Failed));
/// assert!(RunStatus::Completed.can_transition_to(&RunStatus::Completed));
/// assert!(RunStatus::Cancelled.can_transition_to(&RunStatus::Cancelled));
/// ```
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    /// Waiting to be picked up by a worker or inline executor.
    Pending,
    /// Currently executing steps.
    Running,
    /// All steps completed successfully.
    Completed,
    /// A step failed and no retries remain.
    Failed,
    /// A step failed but retries are available; waiting to be re-queued.
    Retrying,
    /// Cancelled by user or API before completion.
    Cancelled,
    /// Waiting for human approval before continuing.
    AwaitingApproval,
}

impl RunStatus {
    /// Returns `true` if the current status can legally transition to `target`.
    ///
    /// Transitioning a terminal state to itself is idempotent (returns `true`).
    pub fn can_transition_to(&self, target: &RunStatus) -> bool {
        if self == target && self.is_terminal() {
            return true;
        }
        matches!(
            (self, target),
            (RunStatus::Pending, RunStatus::Running)
                | (RunStatus::Pending, RunStatus::Cancelled)
                | (RunStatus::Running, RunStatus::Completed)
                | (RunStatus::Running, RunStatus::Failed)
                | (RunStatus::Running, RunStatus::Retrying)
                | (RunStatus::Running, RunStatus::Cancelled)
                | (RunStatus::Running, RunStatus::AwaitingApproval)
                | (RunStatus::Retrying, RunStatus::Running)
                | (RunStatus::Retrying, RunStatus::Failed)
                | (RunStatus::Retrying, RunStatus::Cancelled)
                | (RunStatus::AwaitingApproval, RunStatus::Running)
                | (RunStatus::AwaitingApproval, RunStatus::Failed)
                | (RunStatus::AwaitingApproval, RunStatus::Cancelled)
        )
    }

    /// Returns `true` if this is a terminal state (no further transitions).
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            RunStatus::Completed | RunStatus::Failed | RunStatus::Cancelled
        )
    }
}

impl std::fmt::Display for RunStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunStatus::Pending => f.write_str("Pending"),
            RunStatus::Running => f.write_str("Running"),
            RunStatus::Completed => f.write_str("Completed"),
            RunStatus::Failed => f.write_str("Failed"),
            RunStatus::Retrying => f.write_str("Retrying"),
            RunStatus::Cancelled => f.write_str("Cancelled"),
            RunStatus::AwaitingApproval => f.write_str("AwaitingApproval"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_can_transition_to_running() {
        assert!(RunStatus::Pending.can_transition_to(&RunStatus::Running));
    }

    #[test]
    fn pending_can_transition_to_cancelled() {
        assert!(RunStatus::Pending.can_transition_to(&RunStatus::Cancelled));
    }

    #[test]
    fn pending_cannot_transition_to_completed() {
        assert!(!RunStatus::Pending.can_transition_to(&RunStatus::Completed));
    }

    #[test]
    fn pending_cannot_transition_to_failed() {
        assert!(!RunStatus::Pending.can_transition_to(&RunStatus::Failed));
    }

    #[test]
    fn running_can_transition_to_completed() {
        assert!(RunStatus::Running.can_transition_to(&RunStatus::Completed));
    }

    #[test]
    fn running_can_transition_to_failed() {
        assert!(RunStatus::Running.can_transition_to(&RunStatus::Failed));
    }

    #[test]
    fn running_can_transition_to_retrying() {
        assert!(RunStatus::Running.can_transition_to(&RunStatus::Retrying));
    }

    #[test]
    fn running_can_transition_to_cancelled() {
        assert!(RunStatus::Running.can_transition_to(&RunStatus::Cancelled));
    }

    #[test]
    fn retrying_can_transition_to_running() {
        assert!(RunStatus::Retrying.can_transition_to(&RunStatus::Running));
    }

    #[test]
    fn retrying_can_transition_to_failed() {
        assert!(RunStatus::Retrying.can_transition_to(&RunStatus::Failed));
    }

    #[test]
    fn retrying_can_transition_to_cancelled() {
        assert!(RunStatus::Retrying.can_transition_to(&RunStatus::Cancelled));
    }

    #[test]
    fn completed_is_terminal() {
        assert!(RunStatus::Completed.is_terminal());
        assert!(!RunStatus::Completed.can_transition_to(&RunStatus::Running));
    }

    #[test]
    fn failed_is_terminal() {
        assert!(RunStatus::Failed.is_terminal());
    }

    #[test]
    fn cancelled_is_terminal() {
        assert!(RunStatus::Cancelled.is_terminal());
    }

    #[test]
    fn pending_and_running_not_terminal() {
        assert!(!RunStatus::Pending.is_terminal());
        assert!(!RunStatus::Running.is_terminal());
    }

    #[test]
    fn awaiting_approval_not_terminal() {
        assert!(!RunStatus::AwaitingApproval.is_terminal());
    }

    #[test]
    fn running_can_transition_to_awaiting_approval() {
        assert!(RunStatus::Running.can_transition_to(&RunStatus::AwaitingApproval));
    }

    #[test]
    fn awaiting_approval_can_transition_to_running() {
        assert!(RunStatus::AwaitingApproval.can_transition_to(&RunStatus::Running));
    }

    #[test]
    fn awaiting_approval_can_transition_to_failed() {
        assert!(RunStatus::AwaitingApproval.can_transition_to(&RunStatus::Failed));
    }

    #[test]
    fn awaiting_approval_can_transition_to_cancelled() {
        assert!(RunStatus::AwaitingApproval.can_transition_to(&RunStatus::Cancelled));
    }

    #[test]
    fn awaiting_approval_cannot_transition_to_completed() {
        assert!(!RunStatus::AwaitingApproval.can_transition_to(&RunStatus::Completed));
    }

    #[test]
    fn terminal_to_same_is_idempotent() {
        assert!(RunStatus::Failed.can_transition_to(&RunStatus::Failed));
        assert!(RunStatus::Completed.can_transition_to(&RunStatus::Completed));
        assert!(RunStatus::Cancelled.can_transition_to(&RunStatus::Cancelled));
    }

    #[test]
    fn terminal_to_different_terminal_is_forbidden() {
        assert!(!RunStatus::Failed.can_transition_to(&RunStatus::Completed));
        assert!(!RunStatus::Failed.can_transition_to(&RunStatus::Cancelled));
        assert!(!RunStatus::Completed.can_transition_to(&RunStatus::Failed));
        assert!(!RunStatus::Completed.can_transition_to(&RunStatus::Cancelled));
        assert!(!RunStatus::Cancelled.can_transition_to(&RunStatus::Failed));
        assert!(!RunStatus::Cancelled.can_transition_to(&RunStatus::Completed));
    }

    #[test]
    fn non_terminal_same_state_is_forbidden() {
        assert!(!RunStatus::Pending.can_transition_to(&RunStatus::Pending));
        assert!(!RunStatus::Running.can_transition_to(&RunStatus::Running));
        assert!(!RunStatus::Retrying.can_transition_to(&RunStatus::Retrying));
        assert!(!RunStatus::AwaitingApproval.can_transition_to(&RunStatus::AwaitingApproval));
    }

    #[test]
    fn display() {
        assert_eq!(RunStatus::Pending.to_string(), "Pending");
        assert_eq!(RunStatus::Running.to_string(), "Running");
        assert_eq!(RunStatus::Completed.to_string(), "Completed");
        assert_eq!(RunStatus::Failed.to_string(), "Failed");
        assert_eq!(RunStatus::Retrying.to_string(), "Retrying");
        assert_eq!(RunStatus::Cancelled.to_string(), "Cancelled");
        assert_eq!(RunStatus::AwaitingApproval.to_string(), "AwaitingApproval");
    }

    #[test]
    fn serde_roundtrip() {
        for status in [
            RunStatus::Pending,
            RunStatus::Running,
            RunStatus::Completed,
            RunStatus::Failed,
            RunStatus::Retrying,
            RunStatus::Cancelled,
            RunStatus::AwaitingApproval,
        ] {
            let json = serde_json::to_string(&status).expect("serialize");
            let back: RunStatus = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(status, back);
        }
    }
}
