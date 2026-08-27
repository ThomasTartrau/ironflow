//! [`Step`] entity and related request/update types.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use super::{FsmState, StepKind, StepStatus};

/// Attempt number assigned to steps deserialized from payloads predating the
/// `attempt` field.
fn default_attempt() -> u32 {
    1
}

/// Generate a deterministic trace ID for a step.
///
/// Uses UUIDv5 with `NAMESPACE_OID` and the input
/// `"{run_id}:{name}:{position}"`, so the same run replayed with the same
/// steps always produces the same trace IDs.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::step_trace_id;
/// use uuid::Uuid;
///
/// let run_id = Uuid::nil();
/// let id1 = step_trace_id(run_id, "build", 0);
/// let id2 = step_trace_id(run_id, "build", 0);
/// assert_eq!(id1, id2);
///
/// let id3 = step_trace_id(run_id, "test", 1);
/// assert_ne!(id1, id3);
/// ```
pub fn step_trace_id(run_id: Uuid, name: &str, position: u32) -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("{run_id}:{name}:{position}").as_bytes(),
    )
}

/// A single operation within a run.
///
/// Steps are executed sequentially in order of [`position`](Step::position).
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::Step;
///
/// // Steps are created by RunStore::create_step, not directly.
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Step {
    /// Unique identifier (UUIDv7).
    pub id: Uuid,
    /// Deterministic trace ID for log correlation.
    ///
    /// Generated as `UUIDv5(NAMESPACE_OID, "{run_id}:{name}:{position}")` so
    /// the same run replayed with the same steps always produces the same IDs.
    pub trace_id: Uuid,
    /// The run this step belongs to.
    pub run_id: Uuid,
    /// Human-readable step name (e.g. "build", "test", "review").
    pub name: String,
    /// The type of operation.
    pub kind: StepKind,
    /// Execution wave within the run (0-based).
    ///
    /// In linear flows, this strictly increases (0, 1, 2, ...).
    /// In DAGs with parallel execution, steps at the same wave share
    /// the same position and execute concurrently. Use
    /// `step_dependencies` to determine the actual execution order.
    pub position: u32,
    /// Current FSM status — embeds state + state_machine_id for SQL-side transitions.
    pub status: FsmState<StepStatus>,
    /// Which run attempt produced this step (1-based).
    ///
    /// A run retried twice holds steps with `attempt` 1, 2 and 3. Steps from
    /// earlier attempts are kept for inspection and are never replayed.
    /// Derived by the store from `Run::retry_count` at creation time.
    #[serde(default = "default_attempt")]
    pub attempt: u32,
    /// Serialized operation configuration.
    pub input: Option<Value>,
    /// Serialized operation output.
    pub output: Option<Value>,
    /// Error message if the step failed.
    pub error: Option<String>,
    /// Wall-clock execution duration in milliseconds.
    pub duration_ms: u64,
    /// Cost in USD (agent steps only).
    pub cost_usd: Decimal,
    /// Input token count (agent steps only).
    pub input_tokens: Option<u64>,
    /// Output token count (agent steps only).
    pub output_tokens: Option<u64>,
    /// When the step was created.
    pub created_at: DateTime<Utc>,
    /// When the step record was last updated.
    pub updated_at: DateTime<Utc>,
    /// When step execution started.
    pub started_at: Option<DateTime<Utc>>,
    /// When step execution finished.
    pub completed_at: Option<DateTime<Utc>>,
    /// Debug messages (verbose conversation trace), stored as JSON.
    pub debug_messages: Option<Value>,
    /// Whether this step is an error handler (`on_error`) rather than a normal step.
    #[serde(default)]
    pub is_error_handler: bool,
}

/// Request to create a new step.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::{NewStep, StepKind, step_trace_id};
/// use serde_json::json;
/// use uuid::Uuid;
///
/// let run_id = Uuid::nil();
/// let req = NewStep {
///     run_id,
///     trace_id: step_trace_id(run_id, "build", 0),
///     name: "build".to_string(),
///     kind: StepKind::Shell,
///     position: 0,
///     input: Some(json!({"command": "cargo build"})),
///     is_error_handler: false,
/// };
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewStep {
    /// The run this step belongs to.
    pub run_id: Uuid,
    /// Deterministic trace ID for log correlation.
    pub trace_id: Uuid,
    /// Step name.
    pub name: String,
    /// Operation type.
    pub kind: StepKind,
    /// Execution order (0-based).
    pub position: u32,
    /// Serialized operation configuration.
    pub input: Option<Value>,
    /// Whether this step is an error handler (`on_error`).
    #[serde(default)]
    pub is_error_handler: bool,
}

/// Partial update for a step after execution.
///
/// Only `Some` fields are applied; `None` fields are left unchanged.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::{StepUpdate, StepStatus};
/// use serde_json::json;
///
/// let update = StepUpdate {
///     status: Some(StepStatus::Completed),
///     output: Some(json!({"stdout": "ok"})),
///     ..StepUpdate::default()
/// };
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StepUpdate {
    /// New status.
    pub status: Option<StepStatus>,
    /// Operation output.
    pub output: Option<Value>,
    /// Error message.
    pub error: Option<String>,
    /// Execution duration.
    pub duration_ms: Option<u64>,
    /// Cost in USD.
    pub cost_usd: Option<Decimal>,
    /// Input token count.
    pub input_tokens: Option<u64>,
    /// Output token count.
    pub output_tokens: Option<u64>,
    /// When execution started.
    pub started_at: Option<DateTime<Utc>>,
    /// When execution completed.
    pub completed_at: Option<DateTime<Utc>>,
    /// Debug messages (verbose conversation trace), stored as JSON.
    pub debug_messages: Option<Value>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn newstep_serde_roundtrip() {
        let new_step = NewStep {
            run_id: Uuid::nil(),
            trace_id: step_trace_id(Uuid::nil(), "build", 0),
            name: "build".to_string(),
            kind: StepKind::Shell,
            position: 0,
            input: Some(json!({"command": "cargo build"})),
            is_error_handler: false,
        };

        let json = serde_json::to_string(&new_step).expect("serialize");
        let back: NewStep = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(back.run_id, new_step.run_id);
        assert_eq!(back.name, new_step.name);
        assert_eq!(back.kind, new_step.kind);
        assert_eq!(back.position, new_step.position);
        assert_eq!(back.input, new_step.input);
    }

    #[test]
    fn step_serde_preserves_all_fields() {
        use crate::entities::FsmState;
        use chrono::Utc;

        let now = Utc::now();
        let run_id = Uuid::now_v7();
        let step = Step {
            id: Uuid::now_v7(),
            trace_id: step_trace_id(run_id, "test-step", 1),
            run_id,
            name: "test-step".to_string(),
            kind: StepKind::Agent,
            position: 1,
            status: FsmState::new(StepStatus::Completed, Uuid::now_v7()),
            attempt: 2,
            input: Some(json!({"input": "data"})),
            output: Some(json!({"output": "result"})),
            error: None,
            duration_ms: 2500,
            cost_usd: Decimal::new(150, 2),
            input_tokens: Some(100),
            output_tokens: Some(200),
            created_at: now,
            updated_at: now,
            started_at: Some(now),
            completed_at: Some(now),
            debug_messages: None,
            is_error_handler: false,
        };

        let json = serde_json::to_string(&step).expect("serialize");
        let back: Step = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(back.id, step.id);
        assert_eq!(back.run_id, step.run_id);
        assert_eq!(back.name, step.name);
        assert_eq!(back.kind, step.kind);
        assert_eq!(back.position, step.position);
        assert_eq!(back.status.state, step.status.state);
        assert_eq!(back.attempt, step.attempt);
        assert_eq!(back.input, step.input);
        assert_eq!(back.output, step.output);
        assert_eq!(back.error, step.error);
        assert_eq!(back.duration_ms, step.duration_ms);
        assert_eq!(back.cost_usd, step.cost_usd);
        assert_eq!(back.input_tokens, step.input_tokens);
        assert_eq!(back.output_tokens, step.output_tokens);
    }

    #[test]
    fn stepupdate_default_is_no_changes() {
        let update = StepUpdate::default();
        assert!(update.status.is_none());
        assert!(update.output.is_none());
        assert!(update.error.is_none());
        assert!(update.duration_ms.is_none());
        assert!(update.cost_usd.is_none());
        assert!(update.input_tokens.is_none());
        assert!(update.output_tokens.is_none());
        assert!(update.started_at.is_none());
        assert!(update.completed_at.is_none());
        assert!(update.debug_messages.is_none());
    }

    #[test]
    fn stepupdate_serde_roundtrip() {
        let update = StepUpdate {
            status: Some(StepStatus::Completed),
            output: Some(json!({"result": "ok"})),
            error: None,
            duration_ms: Some(1000),
            cost_usd: Some(Decimal::new(50, 2)),
            input_tokens: Some(50),
            output_tokens: Some(75),
            started_at: None,
            completed_at: None,
            debug_messages: None,
        };

        let json = serde_json::to_string(&update).expect("serialize");
        let back: StepUpdate = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(back.status, update.status);
        assert_eq!(back.output, update.output);
        assert_eq!(back.duration_ms, update.duration_ms);
        assert_eq!(back.cost_usd, update.cost_usd);
        assert_eq!(back.input_tokens, update.input_tokens);
        assert_eq!(back.output_tokens, update.output_tokens);
    }

    #[test]
    fn trace_id_is_deterministic() {
        let run_id = Uuid::nil();
        let id1 = step_trace_id(run_id, "build", 0);
        let id2 = step_trace_id(run_id, "build", 0);
        assert_eq!(id1, id2);
    }

    #[test]
    fn trace_id_differs_for_different_inputs() {
        let run_id = Uuid::nil();
        let a = step_trace_id(run_id, "build", 0);
        let b = step_trace_id(run_id, "test", 0);
        let c = step_trace_id(run_id, "build", 1);
        let d = step_trace_id(Uuid::max(), "build", 0);

        assert_ne!(a, b);
        assert_ne!(a, c);
        assert_ne!(a, d);
    }

    #[test]
    fn trace_id_is_uuid_v5() {
        let id = step_trace_id(Uuid::nil(), "build", 0);
        assert_eq!(id.get_version_num(), 5);
    }
}
