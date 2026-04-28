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
//! - [`RunEvent`] / [`StepEvent`] -- Typed events that trigger transitions.
//! - [`Transition`] — A recorded state change with event and timestamp.

mod run_fsm;
mod step_fsm;

pub use run_fsm::{RunEvent, RunFsm};
pub use step_fsm::{StepEvent, StepFsm};

use std::fmt;

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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
pub struct TransitionError<S: fmt::Display, E: fmt::Display> {
    /// Current state when the invalid transition was attempted.
    pub from: S,
    /// Event that was rejected.
    pub event: E,
}

impl<S: fmt::Display, E: fmt::Display> fmt::Display for TransitionError<S, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid transition: event '{}' not allowed in state '{}'",
            self.event, self.from
        )
    }
}

impl<S: fmt::Debug + fmt::Display, E: fmt::Debug + fmt::Display> std::error::Error
    for TransitionError<S, E>
{
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn transition_captures_state_change() {
        let now = Utc::now();
        let t: Transition<String, String> = Transition {
            from: "Pending".to_string(),
            to: "Running".to_string(),
            event: "picked_up".to_string(),
            at: now,
        };

        assert_eq!(t.from, "Pending");
        assert_eq!(t.to, "Running");
        assert_eq!(t.event, "picked_up");
        assert_eq!(t.at, now);
    }

    #[test]
    fn transition_with_different_types() {
        let now = Utc::now();
        #[derive(Debug, Clone, PartialEq)]
        enum State {
            Pending,
            Running,
            Completed,
        }

        #[derive(Debug, Clone)]
        enum Event {
            Started,
            Finished,
        }

        let t = Transition {
            from: State::Pending,
            to: State::Running,
            event: Event::Started,
            at: now,
        };

        assert_eq!(t.from, State::Pending);
        assert_eq!(t.to, State::Running);

        let t2 = Transition {
            from: State::Running,
            to: State::Completed,
            event: Event::Finished,
            at: now,
        };

        assert_eq!(t2.from, State::Running);
        assert_eq!(t2.to, State::Completed);
    }

    #[test]
    fn transition_serializes_to_json() {
        let now = Utc::now();
        let t: Transition<String, String> = Transition {
            from: "A".to_string(),
            to: "B".to_string(),
            event: "go".to_string(),
            at: now,
        };

        let json = serde_json::to_value(&t).expect("serialize");
        assert_eq!(json["from"], "A");
        assert_eq!(json["to"], "B");
        assert_eq!(json["event"], "go");
    }

    #[test]
    fn transition_deserializes_from_json() {
        let json_str =
            r#"{"from":"pending","to":"running","event":"picked_up","at":"2025-01-01T00:00:00Z"}"#;
        let t: Transition<String, String> = serde_json::from_str(json_str).expect("deserialize");

        assert_eq!(t.from, "pending");
        assert_eq!(t.to, "running");
        assert_eq!(t.event, "picked_up");
    }

    #[test]
    fn transition_error_formats_message() {
        let err: TransitionError<String, String> = TransitionError {
            from: "Completed".to_string(),
            event: "picked_up".to_string(),
        };

        let msg = err.to_string();
        assert!(msg.contains("Completed"));
        assert!(msg.contains("picked_up"));
        assert!(msg.contains("invalid transition"));
    }

    #[test]
    fn transition_error_message_is_informative() {
        let err: TransitionError<&str, &str> = TransitionError {
            from: "Failed",
            event: "retry",
        };

        let msg = err.to_string();
        assert!(msg.contains("Failed"));
        assert!(msg.contains("retry"));
    }

    #[test]
    fn transition_error_implements_error_trait() {
        use std::error::Error;

        let err: TransitionError<String, String> = TransitionError {
            from: "Running".to_string(),
            event: "start".to_string(),
        };

        let error_ref: &dyn Error = &err;
        assert!(!error_ref.to_string().is_empty());
    }

    #[test]
    fn transition_error_clone() {
        let err = TransitionError {
            from: "Pending".to_string(),
            event: "resume".to_string(),
        };

        let cloned = err.clone();
        assert_eq!(cloned.from, err.from);
        assert_eq!(cloned.event, err.event);
    }

    #[test]
    fn transition_clone_preserves_all_fields() {
        let now = Utc::now();
        let t1 = Transition {
            from: "A".to_string(),
            to: "B".to_string(),
            event: "E".to_string(),
            at: now,
        };

        let t2 = t1.clone();
        assert_eq!(t1, t2);
        assert_eq!(t1.at, t2.at);
    }

    #[test]
    fn transition_debug_output_contains_fields() {
        let t: Transition<String, String> = Transition {
            from: "x".to_string(),
            to: "y".to_string(),
            event: "z".to_string(),
            at: Utc::now(),
        };

        let debug = format!("{:?}", t);
        assert!(debug.contains("x"));
        assert!(debug.contains("y"));
        assert!(debug.contains("z"));
    }
}
