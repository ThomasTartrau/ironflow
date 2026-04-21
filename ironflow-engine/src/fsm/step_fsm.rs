//! [`StepFsm`] — Finite state machine for the step lifecycle.

use chrono::Utc;
use ironflow_store::entities::StepStatus;
use serde::{Deserialize, Serialize};
use strum::Display;

use super::{Transition, TransitionError};

/// Events that drive [`StepFsm`] transitions.
///
/// # Examples
///
/// ```
/// use ironflow_engine::fsm::StepEvent;
///
/// let event = StepEvent::Started;
/// assert_eq!(event.to_string(), "started");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum StepEvent {
    /// Execution started.
    Started,
    /// Execution completed successfully.
    Succeeded,
    /// Execution failed.
    Failed,
    /// Step was skipped (prior step failed).
    Skipped,
    /// Step is suspended, waiting for human approval.
    Suspended,
    /// Approval received, step resumes.
    Resumed,
    /// Human rejected the approval.
    Rejected,
}

/// Finite state machine for a workflow step.
///
/// # Transition table
///
/// | From | Event | To |
/// |------|-------|----|
/// | Pending | Started | Running |
/// | Pending | Skipped | Skipped |
/// | Running | Succeeded | Completed |
/// | Running | Failed | Failed |
/// | Running | Suspended | AwaitingApproval |
/// | AwaitingApproval | Resumed | Running |
/// | AwaitingApproval | Rejected | Rejected |
/// | AwaitingApproval | Failed | Failed |
///
/// # Examples
///
/// ```
/// use ironflow_engine::fsm::{StepFsm, StepEvent};
/// use ironflow_store::entities::StepStatus;
///
/// let mut fsm = StepFsm::new();
/// fsm.apply(StepEvent::Started).unwrap();
/// fsm.apply(StepEvent::Succeeded).unwrap();
/// assert_eq!(fsm.state(), StepStatus::Completed);
/// ```
#[derive(Debug, Clone)]
pub struct StepFsm {
    state: StepStatus,
    history: Vec<Transition<StepStatus, StepEvent>>,
}

impl StepFsm {
    /// Create a new FSM in `Pending` state.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::fsm::StepFsm;
    /// use ironflow_store::entities::StepStatus;
    ///
    /// let fsm = StepFsm::new();
    /// assert_eq!(fsm.state(), StepStatus::Pending);
    /// ```
    pub fn new() -> Self {
        Self {
            state: StepStatus::Pending,
            history: Vec::new(),
        }
    }

    /// Create a FSM from an existing state.
    pub fn from_state(state: StepStatus) -> Self {
        Self {
            state,
            history: Vec::new(),
        }
    }

    /// Returns the current state.
    pub fn state(&self) -> StepStatus {
        self.state
    }

    /// Returns the full transition history.
    pub fn history(&self) -> &[Transition<StepStatus, StepEvent>] {
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
    /// use ironflow_engine::fsm::{StepFsm, StepEvent};
    ///
    /// let mut fsm = StepFsm::new();
    /// assert!(fsm.apply(StepEvent::Started).is_ok());
    /// assert!(fsm.apply(StepEvent::Started).is_err()); // can't start twice
    /// ```
    pub fn apply(
        &mut self,
        event: StepEvent,
    ) -> Result<StepStatus, TransitionError<StepStatus, StepEvent>> {
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
    pub fn can_apply(&self, event: StepEvent) -> bool {
        next_state(self.state, event).is_some()
    }
}

impl Default for StepFsm {
    fn default() -> Self {
        Self::new()
    }
}

fn next_state(from: StepStatus, event: StepEvent) -> Option<StepStatus> {
    match (from, event) {
        (StepStatus::Pending, StepEvent::Started) => Some(StepStatus::Running),
        (StepStatus::Pending, StepEvent::Skipped) => Some(StepStatus::Skipped),
        (StepStatus::Running, StepEvent::Succeeded) => Some(StepStatus::Completed),
        (StepStatus::Running, StepEvent::Failed) => Some(StepStatus::Failed),
        (StepStatus::Running, StepEvent::Suspended) => Some(StepStatus::AwaitingApproval),
        (StepStatus::AwaitingApproval, StepEvent::Resumed) => Some(StepStatus::Running),
        (StepStatus::AwaitingApproval, StepEvent::Rejected) => Some(StepStatus::Rejected),
        (StepStatus::AwaitingApproval, StepEvent::Failed) => Some(StepStatus::Failed),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_path() {
        let mut fsm = StepFsm::new();
        fsm.apply(StepEvent::Started).unwrap();
        fsm.apply(StepEvent::Succeeded).unwrap();
        assert_eq!(fsm.state(), StepStatus::Completed);
        assert!(fsm.is_terminal());
        assert_eq!(fsm.history().len(), 2);
    }

    #[test]
    fn failure_path() {
        let mut fsm = StepFsm::new();
        fsm.apply(StepEvent::Started).unwrap();
        fsm.apply(StepEvent::Failed).unwrap();
        assert_eq!(fsm.state(), StepStatus::Failed);
        assert!(fsm.is_terminal());
    }

    #[test]
    fn skip_path() {
        let mut fsm = StepFsm::new();
        fsm.apply(StepEvent::Skipped).unwrap();
        assert_eq!(fsm.state(), StepStatus::Skipped);
        assert!(fsm.is_terminal());
    }

    #[test]
    fn cannot_start_twice() {
        let mut fsm = StepFsm::new();
        fsm.apply(StepEvent::Started).unwrap();
        assert!(fsm.apply(StepEvent::Started).is_err());
    }

    #[test]
    fn cannot_succeed_from_pending() {
        let mut fsm = StepFsm::new();
        assert!(fsm.apply(StepEvent::Succeeded).is_err());
    }

    #[test]
    fn cannot_transition_from_terminal() {
        let mut fsm = StepFsm::new();
        fsm.apply(StepEvent::Started).unwrap();
        fsm.apply(StepEvent::Succeeded).unwrap();
        assert!(fsm.apply(StepEvent::Started).is_err());
        assert!(fsm.apply(StepEvent::Failed).is_err());
    }

    #[test]
    fn can_apply_without_mutation() {
        let fsm = StepFsm::new();
        assert!(fsm.can_apply(StepEvent::Started));
        assert!(fsm.can_apply(StepEvent::Skipped));
        assert!(!fsm.can_apply(StepEvent::Succeeded));
        assert!(!fsm.can_apply(StepEvent::Failed));
    }

    #[test]
    fn from_state_resumes() {
        let mut fsm = StepFsm::from_state(StepStatus::Running);
        assert!(fsm.history().is_empty());
        fsm.apply(StepEvent::Failed).unwrap();
        assert_eq!(fsm.state(), StepStatus::Failed);
    }

    #[test]
    fn history_records_all_transitions() {
        let mut fsm = StepFsm::new();
        fsm.apply(StepEvent::Started).unwrap();
        fsm.apply(StepEvent::Succeeded).unwrap();

        let h = fsm.history();
        assert_eq!(h[0].from, StepStatus::Pending);
        assert_eq!(h[0].to, StepStatus::Running);
        assert_eq!(h[0].event, StepEvent::Started);
        assert_eq!(h[1].from, StepStatus::Running);
        assert_eq!(h[1].to, StepStatus::Completed);
        assert_eq!(h[1].event, StepEvent::Succeeded);
    }

    #[test]
    fn approval_suspend_and_resume_path() {
        let mut fsm = StepFsm::new();
        fsm.apply(StepEvent::Started).unwrap();
        fsm.apply(StepEvent::Suspended).unwrap();
        assert_eq!(fsm.state(), StepStatus::AwaitingApproval);
        assert!(!fsm.is_terminal());

        fsm.apply(StepEvent::Resumed).unwrap();
        assert_eq!(fsm.state(), StepStatus::Running);

        fsm.apply(StepEvent::Succeeded).unwrap();
        assert_eq!(fsm.state(), StepStatus::Completed);
        assert!(fsm.is_terminal());
    }

    #[test]
    fn approval_reject_path() {
        let mut fsm = StepFsm::new();
        fsm.apply(StepEvent::Started).unwrap();
        fsm.apply(StepEvent::Suspended).unwrap();
        assert_eq!(fsm.state(), StepStatus::AwaitingApproval);

        fsm.apply(StepEvent::Rejected).unwrap();
        assert_eq!(fsm.state(), StepStatus::Rejected);
        assert!(fsm.is_terminal());
    }

    #[test]
    fn cannot_suspend_from_pending() {
        let mut fsm = StepFsm::new();
        assert!(fsm.apply(StepEvent::Suspended).is_err());
    }

    #[test]
    fn cannot_resume_from_running() {
        let mut fsm = StepFsm::new();
        fsm.apply(StepEvent::Started).unwrap();
        assert!(fsm.apply(StepEvent::Resumed).is_err());
    }
}
