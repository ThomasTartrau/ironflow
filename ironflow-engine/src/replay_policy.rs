//! Replay eligibility policy for finished runs.
//!
//! `POST /runs/{id}/replay` creates a new run from a previous one, with the
//! same payload, on the handler's **current** version. This module owns the
//! run-level eligibility rule for that route: a run can be replayed only once
//! it reached a terminal state.
//!
//! Unlike [`crate::retry_policy`], which classifies the error behind a failure
//! to decide whether replaying it is worthwhile, this is a pure status check:
//! any final outcome, successful or not, can be replayed.
//!
//! # Examples
//!
//! ```
//! use ironflow_engine::replay_policy::is_run_replayable;
//! use ironflow_store::models::RunStatus;
//!
//! assert!(is_run_replayable(RunStatus::Failed));
//! assert!(!is_run_replayable(RunStatus::Pending));
//! ```

use ironflow_store::models::RunStatus;

/// Returns `true` if a run in this status can be replayed as a new run.
///
/// Only a terminal run -- `Completed`, `Failed`, `Warning` or `Cancelled` --
/// is eligible. A run still in flight (`Pending`, `Running`, `Retrying`,
/// `AwaitingApproval`, `Sleeping`) is rejected: it has not produced a final
/// outcome yet, and the original run itself is the one to watch instead.
///
/// # Examples
///
/// ```
/// use ironflow_engine::replay_policy::is_run_replayable;
/// use ironflow_store::models::RunStatus;
///
/// assert!(is_run_replayable(RunStatus::Completed));
/// assert!(!is_run_replayable(RunStatus::Running));
/// ```
pub fn is_run_replayable(status: RunStatus) -> bool {
    status.is_terminal()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_run_is_replayable() {
        assert!(is_run_replayable(RunStatus::Completed));
    }

    #[test]
    fn failed_run_is_replayable() {
        assert!(is_run_replayable(RunStatus::Failed));
    }

    #[test]
    fn warning_run_is_replayable() {
        assert!(is_run_replayable(RunStatus::Warning));
    }

    #[test]
    fn cancelled_run_is_replayable() {
        assert!(is_run_replayable(RunStatus::Cancelled));
    }

    #[test]
    fn pending_run_is_not_replayable() {
        assert!(!is_run_replayable(RunStatus::Pending));
    }

    #[test]
    fn running_run_is_not_replayable() {
        assert!(!is_run_replayable(RunStatus::Running));
    }

    #[test]
    fn retrying_run_is_not_replayable() {
        assert!(!is_run_replayable(RunStatus::Retrying));
    }

    #[test]
    fn awaiting_approval_run_is_not_replayable() {
        assert!(!is_run_replayable(RunStatus::AwaitingApproval));
    }

    #[test]
    fn sleeping_run_is_not_replayable() {
        assert!(!is_run_replayable(RunStatus::Sleeping));
    }
}
