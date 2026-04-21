//! [`RunFsm`] — Finite state machine for the run lifecycle.
//!
//! Events drive transitions; the FSM rejects invalid ones and keeps
//! a full history of state changes.

use chrono::Utc;
use ironflow_store::entities::RunStatus;
use serde::{Deserialize, Serialize};
use strum::Display;

use super::{Transition, TransitionError};

/// Events that drive [`RunFsm`] transitions.
///
/// Each event represents something that happened during execution.
///
/// # Examples
///
/// ```
/// use ironflow_engine::fsm::RunEvent;
///
/// let event = RunEvent::PickedUp;
/// assert_eq!(event.to_string(), "picked_up");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum RunEvent {
    /// Worker or inline executor picked up the run.
    PickedUp,
    /// All steps completed successfully.
    AllStepsCompleted,
    /// A step failed and the error is not retryable, or max retries exhausted.
    StepFailed,
    /// A step failed but a retry is possible.
    StepFailedRetryable,
    /// Retry attempt started.
    RetryStarted,
    /// Maximum retries exhausted after a retryable failure.
    MaxRetriesExceeded,
    /// User or API requested cancellation.
    CancelRequested,
    /// A step requires human approval before continuing.
    ApprovalRequested,
    /// Human approved the run to continue.
    Approved,
    /// Human rejected the run.
    Rejected,
}

/// Finite state machine for a workflow run.
///
/// Wraps a [`RunStatus`] and enforces valid transitions via typed
/// [`RunEvent`]s. Records every transition in a history log.
///
/// # Transition table
///
/// | From | Event | To |
/// |------|-------|----|
/// | Pending | PickedUp | Running |
/// | Pending | CancelRequested | Cancelled |
/// | Running | AllStepsCompleted | Completed |
/// | Running | StepFailed | Failed |
/// | Running | StepFailedRetryable | Retrying |
/// | Running | CancelRequested | Cancelled |
/// | Retrying | RetryStarted | Running |
/// | Retrying | MaxRetriesExceeded | Failed |
/// | Retrying | CancelRequested | Cancelled |
/// | Running | ApprovalRequested | AwaitingApproval |
/// | AwaitingApproval | Approved | Running |
/// | AwaitingApproval | Rejected | Failed |
/// | AwaitingApproval | CancelRequested | Cancelled |
///
/// # Examples
///
/// ```
/// use ironflow_engine::fsm::{RunFsm, RunEvent};
/// use ironflow_store::entities::RunStatus;
///
/// let mut fsm = RunFsm::new();
/// assert_eq!(fsm.state(), RunStatus::Pending);
///
/// fsm.apply(RunEvent::PickedUp).unwrap();
/// assert_eq!(fsm.state(), RunStatus::Running);
///
/// fsm.apply(RunEvent::AllStepsCompleted).unwrap();
/// assert_eq!(fsm.state(), RunStatus::Completed);
/// assert_eq!(fsm.history().len(), 2);
/// ```
#[derive(Debug, Clone)]
pub struct RunFsm {
    state: RunStatus,
    history: Vec<Transition<RunStatus, RunEvent>>,
}

impl RunFsm {
    /// Create a new FSM in `Pending` state.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::fsm::RunFsm;
    /// use ironflow_store::entities::RunStatus;
    ///
    /// let fsm = RunFsm::new();
    /// assert_eq!(fsm.state(), RunStatus::Pending);
    /// ```
    pub fn new() -> Self {
        Self {
            state: RunStatus::Pending,
            history: Vec::new(),
        }
    }

    /// Create a FSM from an existing state (e.g. loaded from DB).
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::fsm::RunFsm;
    /// use ironflow_store::entities::RunStatus;
    ///
    /// let fsm = RunFsm::from_state(RunStatus::Running);
    /// assert_eq!(fsm.state(), RunStatus::Running);
    /// ```
    pub fn from_state(state: RunStatus) -> Self {
        Self {
            state,
            history: Vec::new(),
        }
    }

    /// Returns the current state.
    pub fn state(&self) -> RunStatus {
        self.state
    }

    /// Returns the full transition history.
    pub fn history(&self) -> &[Transition<RunStatus, RunEvent>] {
        &self.history
    }

    /// Returns `true` if the FSM is in a terminal state.
    pub fn is_terminal(&self) -> bool {
        self.state.is_terminal()
    }

    /// Apply an event, transitioning to a new state if valid.
    ///
    /// # Errors
    ///
    /// Returns [`TransitionError`] if the event is not allowed in the current state.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::fsm::{RunFsm, RunEvent};
    /// use ironflow_store::entities::RunStatus;
    ///
    /// let mut fsm = RunFsm::new();
    ///
    /// // Valid transition
    /// assert!(fsm.apply(RunEvent::PickedUp).is_ok());
    ///
    /// // Invalid: can't pick up a running run
    /// assert!(fsm.apply(RunEvent::PickedUp).is_err());
    /// ```
    pub fn apply(
        &mut self,
        event: RunEvent,
    ) -> Result<RunStatus, TransitionError<RunStatus, RunEvent>> {
        let next = next_state(self.state, event).ok_or(TransitionError {
            from: self.state,
            event,
        })?;

        let transition = Transition {
            from: self.state,
            to: next,
            event,
            at: Utc::now(),
        };

        self.history.push(transition);
        self.state = next;
        Ok(next)
    }

    /// Check if an event would be accepted without applying it.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::fsm::{RunFsm, RunEvent};
    ///
    /// let fsm = RunFsm::new();
    /// assert!(fsm.can_apply(RunEvent::PickedUp));
    /// assert!(!fsm.can_apply(RunEvent::AllStepsCompleted));
    /// ```
    pub fn can_apply(&self, event: RunEvent) -> bool {
        next_state(self.state, event).is_some()
    }
}

impl Default for RunFsm {
    fn default() -> Self {
        Self::new()
    }
}

/// Pure transition function — returns the next state for a given (state, event)
/// pair, or `None` if the transition is invalid.
fn next_state(from: RunStatus, event: RunEvent) -> Option<RunStatus> {
    match (from, event) {
        // Pending
        (RunStatus::Pending, RunEvent::PickedUp) => Some(RunStatus::Running),
        (RunStatus::Pending, RunEvent::CancelRequested) => Some(RunStatus::Cancelled),

        // Running
        (RunStatus::Running, RunEvent::AllStepsCompleted) => Some(RunStatus::Completed),
        (RunStatus::Running, RunEvent::StepFailed) => Some(RunStatus::Failed),
        (RunStatus::Running, RunEvent::StepFailedRetryable) => Some(RunStatus::Retrying),
        (RunStatus::Running, RunEvent::CancelRequested) => Some(RunStatus::Cancelled),

        // Retrying
        (RunStatus::Retrying, RunEvent::RetryStarted) => Some(RunStatus::Running),
        (RunStatus::Retrying, RunEvent::MaxRetriesExceeded) => Some(RunStatus::Failed),
        (RunStatus::Retrying, RunEvent::CancelRequested) => Some(RunStatus::Cancelled),

        // Approval
        (RunStatus::Running, RunEvent::ApprovalRequested) => Some(RunStatus::AwaitingApproval),
        (RunStatus::AwaitingApproval, RunEvent::Approved) => Some(RunStatus::Running),
        (RunStatus::AwaitingApproval, RunEvent::Rejected) => Some(RunStatus::Failed),
        (RunStatus::AwaitingApproval, RunEvent::CancelRequested) => Some(RunStatus::Cancelled),

        // Terminal states and all other combos → invalid
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Happy paths ----

    #[test]
    fn pending_to_running() {
        let mut fsm = RunFsm::new();
        let result = fsm.apply(RunEvent::PickedUp);
        assert!(result.is_ok());
        assert_eq!(fsm.state(), RunStatus::Running);
    }

    #[test]
    fn full_success_path() {
        let mut fsm = RunFsm::new();
        fsm.apply(RunEvent::PickedUp).unwrap();
        fsm.apply(RunEvent::AllStepsCompleted).unwrap();
        assert_eq!(fsm.state(), RunStatus::Completed);
        assert!(fsm.is_terminal());
        assert_eq!(fsm.history().len(), 2);
    }

    #[test]
    fn full_failure_path() {
        let mut fsm = RunFsm::new();
        fsm.apply(RunEvent::PickedUp).unwrap();
        fsm.apply(RunEvent::StepFailed).unwrap();
        assert_eq!(fsm.state(), RunStatus::Failed);
        assert!(fsm.is_terminal());
    }

    #[test]
    fn retry_then_success() {
        let mut fsm = RunFsm::new();
        fsm.apply(RunEvent::PickedUp).unwrap();
        fsm.apply(RunEvent::StepFailedRetryable).unwrap();
        assert_eq!(fsm.state(), RunStatus::Retrying);

        fsm.apply(RunEvent::RetryStarted).unwrap();
        assert_eq!(fsm.state(), RunStatus::Running);

        fsm.apply(RunEvent::AllStepsCompleted).unwrap();
        assert_eq!(fsm.state(), RunStatus::Completed);
        assert_eq!(fsm.history().len(), 4);
    }

    #[test]
    fn retry_then_max_retries_exceeded() {
        let mut fsm = RunFsm::new();
        fsm.apply(RunEvent::PickedUp).unwrap();
        fsm.apply(RunEvent::StepFailedRetryable).unwrap();
        fsm.apply(RunEvent::MaxRetriesExceeded).unwrap();
        assert_eq!(fsm.state(), RunStatus::Failed);
    }

    #[test]
    fn cancel_from_pending() {
        let mut fsm = RunFsm::new();
        fsm.apply(RunEvent::CancelRequested).unwrap();
        assert_eq!(fsm.state(), RunStatus::Cancelled);
        assert!(fsm.is_terminal());
    }

    #[test]
    fn cancel_from_running() {
        let mut fsm = RunFsm::new();
        fsm.apply(RunEvent::PickedUp).unwrap();
        fsm.apply(RunEvent::CancelRequested).unwrap();
        assert_eq!(fsm.state(), RunStatus::Cancelled);
    }

    #[test]
    fn cancel_from_retrying() {
        let mut fsm = RunFsm::new();
        fsm.apply(RunEvent::PickedUp).unwrap();
        fsm.apply(RunEvent::StepFailedRetryable).unwrap();
        fsm.apply(RunEvent::CancelRequested).unwrap();
        assert_eq!(fsm.state(), RunStatus::Cancelled);
    }

    // ---- Invalid transitions ----

    #[test]
    fn cannot_complete_from_pending() {
        let mut fsm = RunFsm::new();
        let result = fsm.apply(RunEvent::AllStepsCompleted);
        assert!(result.is_err());
        assert_eq!(fsm.state(), RunStatus::Pending);
    }

    #[test]
    fn cannot_pick_up_running() {
        let mut fsm = RunFsm::new();
        fsm.apply(RunEvent::PickedUp).unwrap();
        let result = fsm.apply(RunEvent::PickedUp);
        assert!(result.is_err());
    }

    #[test]
    fn cannot_transition_from_terminal() {
        let mut fsm = RunFsm::new();
        fsm.apply(RunEvent::PickedUp).unwrap();
        fsm.apply(RunEvent::AllStepsCompleted).unwrap();

        assert!(fsm.apply(RunEvent::PickedUp).is_err());
        assert!(fsm.apply(RunEvent::CancelRequested).is_err());
        assert!(fsm.apply(RunEvent::StepFailed).is_err());
    }

    // ---- can_apply ----

    #[test]
    fn can_apply_checks_without_mutation() {
        let fsm = RunFsm::new();
        assert!(fsm.can_apply(RunEvent::PickedUp));
        assert!(fsm.can_apply(RunEvent::CancelRequested));
        assert!(!fsm.can_apply(RunEvent::AllStepsCompleted));
        assert!(!fsm.can_apply(RunEvent::StepFailed));
        assert_eq!(fsm.state(), RunStatus::Pending);
    }

    // ---- from_state ----

    #[test]
    fn from_state_resumes_at_given_state() {
        let mut fsm = RunFsm::from_state(RunStatus::Running);
        assert_eq!(fsm.state(), RunStatus::Running);
        assert!(fsm.history().is_empty());

        fsm.apply(RunEvent::AllStepsCompleted).unwrap();
        assert_eq!(fsm.state(), RunStatus::Completed);
    }

    // ---- History ----

    #[test]
    fn history_records_transitions() {
        let mut fsm = RunFsm::new();
        fsm.apply(RunEvent::PickedUp).unwrap();
        fsm.apply(RunEvent::StepFailedRetryable).unwrap();
        fsm.apply(RunEvent::RetryStarted).unwrap();

        let history = fsm.history();
        assert_eq!(history.len(), 3);

        assert_eq!(history[0].from, RunStatus::Pending);
        assert_eq!(history[0].to, RunStatus::Running);
        assert_eq!(history[0].event, RunEvent::PickedUp);

        assert_eq!(history[1].from, RunStatus::Running);
        assert_eq!(history[1].to, RunStatus::Retrying);
        assert_eq!(history[1].event, RunEvent::StepFailedRetryable);

        assert_eq!(history[2].from, RunStatus::Retrying);
        assert_eq!(history[2].to, RunStatus::Running);
        assert_eq!(history[2].event, RunEvent::RetryStarted);
    }

    // ---- Approval transitions ----

    #[test]
    fn running_to_awaiting_approval() {
        let mut fsm = RunFsm::new();
        fsm.apply(RunEvent::PickedUp).unwrap();
        fsm.apply(RunEvent::ApprovalRequested).unwrap();
        assert_eq!(fsm.state(), RunStatus::AwaitingApproval);
        assert!(!fsm.is_terminal());
    }

    #[test]
    fn awaiting_approval_approved_resumes_running() {
        let mut fsm = RunFsm::new();
        fsm.apply(RunEvent::PickedUp).unwrap();
        fsm.apply(RunEvent::ApprovalRequested).unwrap();
        fsm.apply(RunEvent::Approved).unwrap();
        assert_eq!(fsm.state(), RunStatus::Running);
    }

    #[test]
    fn awaiting_approval_rejected_fails() {
        let mut fsm = RunFsm::new();
        fsm.apply(RunEvent::PickedUp).unwrap();
        fsm.apply(RunEvent::ApprovalRequested).unwrap();
        fsm.apply(RunEvent::Rejected).unwrap();
        assert_eq!(fsm.state(), RunStatus::Failed);
        assert!(fsm.is_terminal());
    }

    #[test]
    fn awaiting_approval_cancel() {
        let mut fsm = RunFsm::new();
        fsm.apply(RunEvent::PickedUp).unwrap();
        fsm.apply(RunEvent::ApprovalRequested).unwrap();
        fsm.apply(RunEvent::CancelRequested).unwrap();
        assert_eq!(fsm.state(), RunStatus::Cancelled);
        assert!(fsm.is_terminal());
    }

    #[test]
    fn cannot_approve_from_pending() {
        let mut fsm = RunFsm::new();
        assert!(fsm.apply(RunEvent::Approved).is_err());
    }

    #[test]
    fn approval_then_complete() {
        let mut fsm = RunFsm::new();
        fsm.apply(RunEvent::PickedUp).unwrap();
        fsm.apply(RunEvent::ApprovalRequested).unwrap();
        fsm.apply(RunEvent::Approved).unwrap();
        fsm.apply(RunEvent::AllStepsCompleted).unwrap();
        assert_eq!(fsm.state(), RunStatus::Completed);
        assert_eq!(fsm.history().len(), 4);
    }

    // ---- TransitionError Display ----

    #[test]
    fn transition_error_display() {
        let mut fsm = RunFsm::new();
        let err = fsm.apply(RunEvent::AllStepsCompleted).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("all_steps_completed"));
        assert!(msg.contains("Pending"));
    }
}
