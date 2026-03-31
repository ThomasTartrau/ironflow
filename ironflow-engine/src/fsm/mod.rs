//! Finite State Machines for workflow run and step lifecycles.
//!
//! Instead of ad-hoc `can_transition_to()` checks scattered across the codebase,
//! these FSMs provide typed events, explicit transition tables, and a history
//! of transitions for observability.
//!
//! # Architecture
//!
//! - [`RunFsm`] — Manages the lifecycle of a [`Run`](ironflow_store::entities::Run).
//! - [`StepFsm`] — Manages the lifecycle of a [`Step`](ironflow_store::entities::Step).
//! - [`FsmEvent`] — Typed events that trigger transitions.
//! - [`Transition`] — A recorded state change with event and timestamp.

mod run_fsm;
mod step_fsm;

pub use run_fsm::{RunEvent, RunFsm};
pub use step_fsm::{StepEvent, StepFsm};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A recorded state transition.
///
/// Captures the from/to states, the event that triggered the transition,
/// and when it occurred.
///
/// # Examples
///
/// ```
/// use ironflow_engine::fsm::Transition;
///
/// let t: Transition<String, String> = Transition {
///     from: "Pending".to_string(),
///     to: "Running".to_string(),
///     event: "picked_up".to_string(),
///     at: chrono::Utc::now(),
/// };
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transition<S, E> {
    /// State before the transition.
    pub from: S,
    /// State after the transition.
    pub to: S,
    /// Event that triggered the transition.
    pub event: E,
    /// When the transition occurred.
    pub at: DateTime<Utc>,
}

/// Error returned when a transition is not allowed.
///
/// # Examples
///
/// ```
/// use ironflow_engine::fsm::TransitionError;
///
/// let err: TransitionError<String, String> = TransitionError {
///     from: "Completed".to_string(),
///     event: "picked_up".to_string(),
/// };
/// assert!(err.to_string().contains("Completed"));
/// ```
#[derive(Debug, Clone)]
pub struct TransitionError<S: std::fmt::Display, E: std::fmt::Display> {
    /// Current state when the invalid transition was attempted.
    pub from: S,
    /// Event that was rejected.
    pub event: E,
}

impl<S: std::fmt::Display, E: std::fmt::Display> std::fmt::Display for TransitionError<S, E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "invalid transition: event '{}' not allowed in state '{}'",
            self.event, self.from
        )
    }
}

impl<S: std::fmt::Debug + std::fmt::Display, E: std::fmt::Debug + std::fmt::Display>
    std::error::Error for TransitionError<S, E>
{
}
