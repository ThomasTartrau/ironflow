//! Per-workflow broadcast event bus for real-time monitoring.
//!
//! [`WorkflowEventBus`] maintains one [`tokio::sync::broadcast`] channel per
//! workflow run. Consumers (dashboards, SSE routes) subscribe to a specific
//! `run_id` and receive only the events for that run.
//!
//! # Architecture
//!
//! - [`WorkflowEvent`] -- granular step-level events (started, completed,
//!   failed, approval, token usage).
//! - [`WorkflowEventBus`] -- per-run broadcast channels with subscribe /
//!   publish / remove lifecycle.
//!
//! # Examples
//!
//! ```
//! use ironflow_engine::notify::{WorkflowEvent, WorkflowEventBus, WorkflowStepStartedEvent};
//! use uuid::Uuid;
//! use chrono::Utc;
//!
//! let bus = WorkflowEventBus::new();
//! let run_id = Uuid::now_v7();
//!
//! let mut rx = bus.subscribe(run_id);
//!
//! bus.publish(run_id, WorkflowEvent::StepStarted(WorkflowStepStartedEvent {
//!     step_name: "build".to_string(),
//!     step_index: 0,
//!     timestamp: Utc::now(),
//! }));
//! ```

use std::collections::HashMap;
use std::sync::RwLock;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use uuid::Uuid;

/// Default broadcast channel buffer size per run.
const DEFAULT_BUFFER_SIZE: usize = 64;

/// Payload of the `WorkflowEvent::StepStarted` workflow event.
///
/// # Examples
///
/// ```
/// use chrono::Utc;
/// use ironflow_engine::notify::WorkflowStepStartedEvent;
///
/// let payload = WorkflowStepStartedEvent {
///     step_name: "build".to_string(),
///     step_index: 0,
///     timestamp: Utc::now(),
/// };
/// assert_eq!(payload.step_index, 0);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WorkflowStepStartedEvent {
    /// Human-readable step name.
    pub step_name: String,
    /// Zero-based position in the workflow.
    pub step_index: u32,
    /// When the step started.
    pub timestamp: DateTime<Utc>,
}

/// Payload of the `WorkflowEvent::StepCompleted` workflow event.
///
/// # Examples
///
/// ```
/// use ironflow_engine::notify::WorkflowStepCompletedEvent;
///
/// let payload = WorkflowStepCompletedEvent {
///     step_name: "deploy".to_string(),
///     step_index: 1,
///     duration_ms: 5000,
///     output_summary: Some("deployed v1.2.3".to_string()),
/// };
/// assert_eq!(payload.duration_ms, 5000);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WorkflowStepCompletedEvent {
    /// Human-readable step name.
    pub step_name: String,
    /// Zero-based position in the workflow.
    pub step_index: u32,
    /// Step duration in milliseconds.
    pub duration_ms: u64,
    /// Optional summary of the step output.
    pub output_summary: Option<String>,
}

/// Payload of the `WorkflowEvent::StepFailed` workflow event.
///
/// # Examples
///
/// ```
/// use ironflow_engine::notify::WorkflowStepFailedEvent;
///
/// let payload = WorkflowStepFailedEvent {
///     step_name: "test".to_string(),
///     step_index: 2,
///     error: "exit code 1".to_string(),
///     duration_ms: 3000,
/// };
/// assert_eq!(payload.error, "exit code 1");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WorkflowStepFailedEvent {
    /// Human-readable step name.
    pub step_name: String,
    /// Zero-based position in the workflow.
    pub step_index: u32,
    /// Error description.
    pub error: String,
    /// Step duration in milliseconds.
    pub duration_ms: u64,
}

/// Payload of the `WorkflowEvent::ApprovalRequired` workflow event.
///
/// # Examples
///
/// ```
/// use ironflow_engine::notify::WorkflowApprovalRequiredEvent;
/// use uuid::Uuid;
///
/// let payload = WorkflowApprovalRequiredEvent {
///     step_name: "prod-gate".to_string(),
///     step_index: 3,
///     approval_id: Uuid::now_v7(),
/// };
/// assert_eq!(payload.step_name, "prod-gate");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WorkflowApprovalRequiredEvent {
    /// Human-readable step name.
    pub step_name: String,
    /// Zero-based position in the workflow.
    pub step_index: u32,
    /// Identifier of the approval gate.
    pub approval_id: Uuid,
}

/// Payload of the `WorkflowEvent::AgentStepTokensUsed` workflow event.
///
/// # Examples
///
/// ```
/// use ironflow_engine::notify::WorkflowAgentStepTokensUsedEvent;
/// use rust_decimal::Decimal;
///
/// let payload = WorkflowAgentStepTokensUsedEvent {
///     step_name: "review".to_string(),
///     tokens: 15_000,
///     cost_usd: Decimal::new(42, 4),
/// };
/// assert_eq!(payload.tokens, 15_000);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WorkflowAgentStepTokensUsedEvent {
    /// Human-readable step name.
    pub step_name: String,
    /// Total tokens consumed.
    pub tokens: u64,
    /// Estimated cost in USD.
    pub cost_usd: Decimal,
}

/// A granular step-level event for real-time workflow monitoring.
///
/// Unlike [`Event`](super::Event) which covers the full system lifecycle
/// (runs, auth, audit), `WorkflowEvent` tracks individual step transitions
/// within a single run. Serialized with a `type` discriminant for UI
/// consumption.
///
/// Each variant wraps a dedicated payload struct; the serialized form stays
/// flat, with `type` sitting next to the payload fields.
///
/// # Examples
///
/// ```
/// use ironflow_engine::notify::{WorkflowEvent, WorkflowStepStartedEvent};
/// use chrono::Utc;
///
/// let event = WorkflowEvent::StepStarted(WorkflowStepStartedEvent {
///     step_name: "deploy".to_string(),
///     step_index: 0,
///     timestamp: Utc::now(),
/// });
/// assert_eq!(event.event_type(), "step_started");
///
/// let json = serde_json::to_string(&event)?;
/// assert!(json.contains("\"type\":\"step_started\""));
/// # Ok::<(), serde_json::Error>(())
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkflowEvent {
    /// A step began execution.
    StepStarted(WorkflowStepStartedEvent),

    /// A step completed successfully.
    StepCompleted(WorkflowStepCompletedEvent),

    /// A step failed.
    StepFailed(WorkflowStepFailedEvent),

    /// A step requires human approval before the run can continue.
    ApprovalRequired(WorkflowApprovalRequiredEvent),

    /// Token usage report for an agent step.
    AgentStepTokensUsed(WorkflowAgentStepTokensUsedEvent),
}

impl WorkflowEvent {
    /// Event type constant for [`StepStarted`](WorkflowEvent::StepStarted).
    pub const STEP_STARTED: &'static str = "step_started";
    /// Event type constant for [`StepCompleted`](WorkflowEvent::StepCompleted).
    pub const STEP_COMPLETED: &'static str = "step_completed";
    /// Event type constant for [`StepFailed`](WorkflowEvent::StepFailed).
    pub const STEP_FAILED: &'static str = "step_failed";
    /// Event type constant for [`ApprovalRequired`](WorkflowEvent::ApprovalRequired).
    pub const APPROVAL_REQUIRED: &'static str = "approval_required";
    /// Event type constant for [`AgentStepTokensUsed`](WorkflowEvent::AgentStepTokensUsed).
    pub const AGENT_STEP_TOKENS_USED: &'static str = "agent_step_tokens_used";

    /// Returns the event type as a static string (e.g. `"step_started"`).
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::notify::{WorkflowEvent, WorkflowStepStartedEvent};
    /// use chrono::Utc;
    ///
    /// let event = WorkflowEvent::StepStarted(WorkflowStepStartedEvent {
    ///     step_name: "build".to_string(),
    ///     step_index: 0,
    ///     timestamp: Utc::now(),
    /// });
    /// assert_eq!(event.event_type(), "step_started");
    /// ```
    #[deny(unreachable_patterns)]
    pub fn event_type(&self) -> &'static str {
        match self {
            WorkflowEvent::StepStarted(_) => Self::STEP_STARTED,
            WorkflowEvent::StepCompleted(_) => Self::STEP_COMPLETED,
            WorkflowEvent::StepFailed(_) => Self::STEP_FAILED,
            WorkflowEvent::ApprovalRequired(_) => Self::APPROVAL_REQUIRED,
            WorkflowEvent::AgentStepTokensUsed(_) => Self::AGENT_STEP_TOKENS_USED,
        }
    }
}

/// Per-workflow broadcast event bus for real-time monitoring.
///
/// Maintains one [`tokio::sync::broadcast`] channel per workflow run.
/// Consumers call [`subscribe`](Self::subscribe) to receive events for a
/// specific run; producers call [`publish`](Self::publish) to broadcast
/// an event to all subscribers of that run.
///
/// Thread-safe and cheaply cloneable (`Clone` shares the same inner state).
///
/// # Examples
///
/// ```
/// use ironflow_engine::notify::{WorkflowEvent, WorkflowEventBus, WorkflowStepStartedEvent};
/// use uuid::Uuid;
/// use chrono::Utc;
///
/// let bus = WorkflowEventBus::new();
/// let run_id = Uuid::now_v7();
///
/// let mut rx = bus.subscribe(run_id);
/// bus.publish(run_id, WorkflowEvent::StepStarted(WorkflowStepStartedEvent {
///     step_name: "build".to_string(),
///     step_index: 0,
///     timestamp: Utc::now(),
/// }));
/// ```
#[derive(Clone)]
pub struct WorkflowEventBus {
    channels: std::sync::Arc<RwLock<HashMap<Uuid, broadcast::Sender<WorkflowEvent>>>>,
}

impl WorkflowEventBus {
    /// Create a new empty event bus.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::notify::WorkflowEventBus;
    ///
    /// let bus = WorkflowEventBus::new();
    /// ```
    pub fn new() -> Self {
        Self {
            channels: std::sync::Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Subscribe to events for a specific workflow run.
    ///
    /// If no channel exists for this `run_id`, one is created on demand.
    /// Returns a broadcast receiver that yields [`WorkflowEvent`]s for
    /// that run only.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::notify::WorkflowEventBus;
    /// use uuid::Uuid;
    ///
    /// let bus = WorkflowEventBus::new();
    /// let run_id = Uuid::now_v7();
    /// let _rx = bus.subscribe(run_id);
    /// ```
    pub fn subscribe(&self, run_id: Uuid) -> broadcast::Receiver<WorkflowEvent> {
        let mut channels = self.channels.write().expect("event bus lock poisoned");
        let sender = channels
            .entry(run_id)
            .or_insert_with(|| broadcast::channel(DEFAULT_BUFFER_SIZE).0);
        sender.subscribe()
    }

    /// Broadcast an event to all subscribers of a specific workflow run.
    ///
    /// If no channel exists for `run_id` (no subscriber has called
    /// [`subscribe`](Self::subscribe)), the event is silently dropped.
    /// If subscribers exist but none are actively listening, the send
    /// error is ignored.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::notify::{WorkflowEvent, WorkflowEventBus, WorkflowStepStartedEvent};
    /// use uuid::Uuid;
    /// use chrono::Utc;
    ///
    /// let bus = WorkflowEventBus::new();
    /// let run_id = Uuid::now_v7();
    ///
    /// // No subscriber -- silently dropped.
    /// bus.publish(run_id, WorkflowEvent::StepStarted(WorkflowStepStartedEvent {
    ///     step_name: "build".to_string(),
    ///     step_index: 0,
    ///     timestamp: Utc::now(),
    /// }));
    /// ```
    pub fn publish(&self, run_id: Uuid, event: WorkflowEvent) {
        let channels = self.channels.read().expect("event bus lock poisoned");
        if let Some(sender) = channels.get(&run_id) {
            let _ = sender.send(event);
        }
    }

    /// Remove the channel for a workflow run.
    ///
    /// Call this when a run completes or is cleaned up to free resources.
    /// If no channel exists for `run_id`, this is a no-op.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::notify::WorkflowEventBus;
    /// use uuid::Uuid;
    ///
    /// let bus = WorkflowEventBus::new();
    /// let run_id = Uuid::now_v7();
    /// let _rx = bus.subscribe(run_id);
    /// bus.remove(run_id);
    /// ```
    pub fn remove(&self, run_id: Uuid) {
        let mut channels = self.channels.write().expect("event bus lock poisoned");
        channels.remove(&run_id);
    }
}

impl Default for WorkflowEventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for WorkflowEventBus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let count = self.channels.read().map(|c| c.len()).unwrap_or(0);
        f.debug_struct("WorkflowEventBus")
            .field("active_channels", &count)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step_started(step_name: &str) -> WorkflowEvent {
        WorkflowEvent::StepStarted(WorkflowStepStartedEvent {
            step_name: step_name.to_string(),
            step_index: 0,
            timestamp: Utc::now(),
        })
    }

    #[tokio::test]
    async fn subscribe_receives_published_events() {
        let bus = WorkflowEventBus::new();
        let run_id = Uuid::now_v7();

        let mut rx = bus.subscribe(run_id);

        bus.publish(run_id, step_started("build"));

        let received = rx.recv().await.expect("should receive event");
        assert_eq!(received.event_type(), "step_started");
        match received {
            WorkflowEvent::StepStarted(e) => {
                assert_eq!(e.step_name, "build");
                assert_eq!(e.step_index, 0);
            }
            _ => panic!("expected StepStarted"),
        }
    }

    #[test]
    fn subscribe_creates_channel_on_demand() {
        let bus = WorkflowEventBus::new();
        let run_id = Uuid::now_v7();

        let count_before = bus.channels.read().unwrap().len();
        assert_eq!(count_before, 0);

        let _rx = bus.subscribe(run_id);

        let count_after = bus.channels.read().unwrap().len();
        assert_eq!(count_after, 1);
    }

    #[test]
    fn publish_unknown_run_is_noop() {
        let bus = WorkflowEventBus::new();
        let unknown_run = Uuid::now_v7();

        bus.publish(unknown_run, step_started("build"));
    }

    #[test]
    fn remove_cleans_up_channel() {
        let bus = WorkflowEventBus::new();
        let run_id = Uuid::now_v7();

        let _rx = bus.subscribe(run_id);
        assert_eq!(bus.channels.read().unwrap().len(), 1);

        bus.remove(run_id);
        assert_eq!(bus.channels.read().unwrap().len(), 0);
    }

    #[test]
    fn remove_unknown_is_noop() {
        let bus = WorkflowEventBus::new();
        bus.remove(Uuid::now_v7());
    }

    #[test]
    fn workflow_event_serde_roundtrip() {
        let cases: Vec<WorkflowEvent> = vec![
            WorkflowEvent::StepStarted(WorkflowStepStartedEvent {
                step_name: "build".to_string(),
                step_index: 0,
                timestamp: Utc::now(),
            }),
            WorkflowEvent::StepCompleted(WorkflowStepCompletedEvent {
                step_name: "deploy".to_string(),
                step_index: 1,
                duration_ms: 5000,
                output_summary: Some("deployed v1.2.3".to_string()),
            }),
            WorkflowEvent::StepFailed(WorkflowStepFailedEvent {
                step_name: "test".to_string(),
                step_index: 2,
                error: "exit code 1".to_string(),
                duration_ms: 3000,
            }),
            WorkflowEvent::ApprovalRequired(WorkflowApprovalRequiredEvent {
                step_name: "prod-gate".to_string(),
                step_index: 3,
                approval_id: Uuid::now_v7(),
            }),
            WorkflowEvent::AgentStepTokensUsed(WorkflowAgentStepTokensUsedEvent {
                step_name: "review".to_string(),
                tokens: 15000,
                cost_usd: Decimal::new(42, 4),
            }),
        ];

        for event in &cases {
            let json = serde_json::to_string(event).expect("serialize");
            let back: WorkflowEvent = serde_json::from_str(&json).expect("deserialize");

            assert_eq!(back.event_type(), event.event_type());
            assert!(json.contains(&format!("\"type\":\"{}\"", event.event_type())));
        }
    }

    /// The pre-refactor wire format used flat inline-struct variants. Newtype
    /// variants produce and accept the same JSON, so SSE consumers and stored
    /// payloads need no migration.
    #[test]
    fn workflow_event_legacy_flat_json_deserializes() {
        let approval_id: Uuid = "01890000-0000-7000-8000-000000000002"
            .parse()
            .expect("valid uuid");

        let raw = r#"{"type":"step_started","step_name":"build","step_index":0,"timestamp":"2026-01-01T00:00:00Z"}"#;
        match serde_json::from_str::<WorkflowEvent>(raw).expect("legacy payload") {
            WorkflowEvent::StepStarted(e) => {
                assert_eq!(e.step_name, "build");
                assert_eq!(e.step_index, 0);
            }
            other => panic!("expected StepStarted, got {other:?}"),
        }

        let raw = r#"{"type":"step_completed","step_name":"deploy","step_index":1,"duration_ms":5000,"output_summary":"deployed v1.2.3"}"#;
        match serde_json::from_str::<WorkflowEvent>(raw).expect("legacy payload") {
            WorkflowEvent::StepCompleted(e) => {
                assert_eq!(e.duration_ms, 5000);
                assert_eq!(e.output_summary.as_deref(), Some("deployed v1.2.3"));
            }
            other => panic!("expected StepCompleted, got {other:?}"),
        }

        let raw = r#"{"type":"step_failed","step_name":"test","step_index":2,"error":"exit code 1","duration_ms":3000}"#;
        match serde_json::from_str::<WorkflowEvent>(raw).expect("legacy payload") {
            WorkflowEvent::StepFailed(e) => {
                assert_eq!(e.error, "exit code 1");
                assert_eq!(e.duration_ms, 3000);
            }
            other => panic!("expected StepFailed, got {other:?}"),
        }

        let raw = r#"{"type":"approval_required","step_name":"prod-gate","step_index":3,"approval_id":"01890000-0000-7000-8000-000000000002"}"#;
        match serde_json::from_str::<WorkflowEvent>(raw).expect("legacy payload") {
            WorkflowEvent::ApprovalRequired(e) => {
                assert_eq!(e.approval_id, approval_id);
            }
            other => panic!("expected ApprovalRequired, got {other:?}"),
        }

        let raw = r#"{"type":"agent_step_tokens_used","step_name":"review","tokens":15000,"cost_usd":0.5}"#;
        match serde_json::from_str::<WorkflowEvent>(raw).expect("legacy payload") {
            WorkflowEvent::AgentStepTokensUsed(e) => {
                assert_eq!(e.tokens, 15000);
                assert_eq!(e.cost_usd, Decimal::new(5, 1));
            }
            other => panic!("expected AgentStepTokensUsed, got {other:?}"),
        }
    }

    /// Guards the internally-tagged representation: payload fields must stay
    /// siblings of `type`, never nested under a variant key.
    #[test]
    fn serialized_workflow_event_is_flat_with_type_tag() {
        let event = WorkflowEvent::StepFailed(WorkflowStepFailedEvent {
            step_name: "test".to_string(),
            step_index: 2,
            error: "exit code 1".to_string(),
            duration_ms: 3000,
        });

        let value: serde_json::Value = serde_json::to_value(&event).expect("serialize");
        let object = value.as_object().expect("event serializes to an object");

        assert_eq!(
            object.get("type").and_then(|v| v.as_str()),
            Some("step_failed")
        );
        assert_eq!(
            object.get("step_name").and_then(|v| v.as_str()),
            Some("test")
        );
        assert_eq!(object.get("step_index").and_then(|v| v.as_u64()), Some(2));
        assert_eq!(
            object.get("error").and_then(|v| v.as_str()),
            Some("exit code 1")
        );
        assert_eq!(
            object.get("duration_ms").and_then(|v| v.as_u64()),
            Some(3000)
        );
        assert_eq!(object.len(), 5, "no nesting: {object:?}");
    }

    #[test]
    fn event_type_all_variants() {
        let cases: Vec<(WorkflowEvent, &str)> = vec![
            (
                WorkflowEvent::StepStarted(WorkflowStepStartedEvent {
                    step_name: "s".to_string(),
                    step_index: 0,
                    timestamp: Utc::now(),
                }),
                "step_started",
            ),
            (
                WorkflowEvent::StepCompleted(WorkflowStepCompletedEvent {
                    step_name: "s".to_string(),
                    step_index: 0,
                    duration_ms: 0,
                    output_summary: None,
                }),
                "step_completed",
            ),
            (
                WorkflowEvent::StepFailed(WorkflowStepFailedEvent {
                    step_name: "s".to_string(),
                    step_index: 0,
                    error: "e".to_string(),
                    duration_ms: 0,
                }),
                "step_failed",
            ),
            (
                WorkflowEvent::ApprovalRequired(WorkflowApprovalRequiredEvent {
                    step_name: "s".to_string(),
                    step_index: 0,
                    approval_id: Uuid::now_v7(),
                }),
                "approval_required",
            ),
            (
                WorkflowEvent::AgentStepTokensUsed(WorkflowAgentStepTokensUsedEvent {
                    step_name: "s".to_string(),
                    tokens: 0,
                    cost_usd: Decimal::ZERO,
                }),
                "agent_step_tokens_used",
            ),
        ];

        for (event, expected) in cases {
            assert_eq!(event.event_type(), expected);
        }
    }

    #[tokio::test]
    async fn multiple_subscribers_receive_same_event() {
        let bus = WorkflowEventBus::new();
        let run_id = Uuid::now_v7();

        let mut rx1 = bus.subscribe(run_id);
        let mut rx2 = bus.subscribe(run_id);

        bus.publish(run_id, step_started("build"));

        let e1 = rx1.recv().await.expect("rx1 should receive");
        let e2 = rx2.recv().await.expect("rx2 should receive");

        assert_eq!(e1.event_type(), "step_started");
        assert_eq!(e2.event_type(), "step_started");
    }

    #[tokio::test]
    async fn events_isolated_between_runs() {
        let bus = WorkflowEventBus::new();
        let run_a = Uuid::now_v7();
        let run_b = Uuid::now_v7();

        let mut rx_a = bus.subscribe(run_a);
        let mut rx_b = bus.subscribe(run_b);

        bus.publish(run_a, step_started("only-for-a"));

        let received = rx_a.recv().await.expect("rx_a should receive");
        match received {
            WorkflowEvent::StepStarted(e) => {
                assert_eq!(e.step_name, "only-for-a");
            }
            _ => panic!("expected StepStarted"),
        }

        // rx_b should have nothing -- try_recv returns Empty.
        assert!(rx_b.try_recv().is_err());
    }
}
