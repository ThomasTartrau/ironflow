//! [`FsmState`] — Embeds the current state alongside its state machine ID.
//!
//! This avoids having two separate fields for the same concept. The
//! `state_machine_id` is needed for SQL-side transitions via `lib_fsm`,
//! while `state` is the Rust-side enum for pattern matching.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A state value paired with its `lib_fsm` state machine instance ID.
///
/// Modeled after Netir's `FsmState<T>` pattern. Keeps the FSM instance
/// reference together with the resolved state enum so callers don't need
/// to track both separately.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::{FsmState, RunStatus};
/// use uuid::Uuid;
///
/// let status = FsmState {
///     state: RunStatus::Pending,
///     state_machine_id: Uuid::nil(),
/// };
/// assert_eq!(status.state, RunStatus::Pending);
/// ```
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct FsmState<T: Clone + Copy> {
    /// The resolved state enum value.
    pub state: T,
    /// The `lib_fsm.state_machine.state_machine__id` for SQL-side transitions.
    pub state_machine_id: Uuid,
}

impl<T: Clone + Copy> FsmState<T> {
    /// Create a new `FsmState`.
    pub fn new(state: T, state_machine_id: Uuid) -> Self {
        Self {
            state,
            state_machine_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::RunStatus;

    #[test]
    fn new_creates_with_values() {
        let id = Uuid::nil();
        let fsm = FsmState::new(RunStatus::Running, id);
        assert_eq!(fsm.state, RunStatus::Running);
        assert_eq!(fsm.state_machine_id, id);
    }

    #[test]
    fn serde_roundtrip() {
        let fsm = FsmState::new(RunStatus::Pending, Uuid::nil());
        let json = serde_json::to_string(&fsm).expect("serialize");
        let back: FsmState<RunStatus> = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(fsm, back);
    }
}
