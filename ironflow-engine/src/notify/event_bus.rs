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
//! use ironflow_engine::notify::{WorkflowEventBus, WorkflowEvent};
//! use uuid::Uuid;
//! use chrono::Utc;
//!
//! let bus = WorkflowEventBus::new();
//! let run_id = Uuid::now_v7();
//!
//! let mut rx = bus.subscribe(run_id);
//!
//! bus.publish(run_id, WorkflowEvent::StepStarted {
//!     step_name: "build".to_string(),
//!     step_index: 0,
//!     timestamp: Utc::now(),
//! });
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

/// A granular step-level event for real-time workflow monitoring.
///
/// Unlike [`Event`](super::Event) which covers the full system lifecycle
/// (runs, auth, audit), `WorkflowEvent` tracks individual step transitions
/// within a single run. Serialized with a `type` discriminant for UI
/// consumption.
///
/// # Examples
///
/// ```
/// use ironflow_engine::notify::WorkflowEvent;
/// use chrono::Utc;
///
/// let event = WorkflowEvent::StepStarted {
///     step_name: "deploy".to_string(),
///     step_index: 0,
///     timestamp: Utc::now(),
/// };
/// assert_eq!(event.event_type(), "step_started");
///
/// let json = serde_json::to_string(&event).unwrap();
/// assert!(json.contains("\"type\":\"step_started\""));
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkflowEvent {
    /// A step began execution.
    StepStarted {
        /// Human-readable step name.
        step_name: String,
        /// Zero-based position in the workflow.
        step_index: u32,
        /// When the step started.
        timestamp: DateTime<Utc>,
    },

    /// A step completed successfully.
    StepCompleted {
        /// Human-readable step name.
        step_name: String,
        /// Zero-based position in the workflow.
        step_index: u32,
        /// Step duration in milliseconds.
        duration_ms: u64,
        /// Optional summary of the step output.
        output_summary: Option<String>,
    },

    /// A step failed.
    StepFailed {
        /// Human-readable step name.
        step_name: String,
        /// Zero-based position in the workflow.
        step_index: u32,
        /// Error description.
        error: String,
        /// Step duration in milliseconds.
        duration_ms: u64,
    },

    /// A step requires human approval before the run can continue.
    ApprovalRequired {
        /// Human-readable step name.
        step_name: String,
        /// Zero-based position in the workflow.
        step_index: u32,
        /// Identifier of the approval gate.
        approval_id: Uuid,
    },

    /// Token usage report for an agent step.
    AgentStepTokensUsed {
        /// Human-readable step name.
        step_name: String,
        /// Total tokens consumed.
        tokens: u64,
        /// Estimated cost in USD.
        cost_usd: Decimal,
    },
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
    /// use ironflow_engine::notify::WorkflowEvent;
    /// use chrono::Utc;
    ///
    /// let event = WorkflowEvent::StepStarted {
    ///     step_name: "build".to_string(),
    ///     step_index: 0,
    ///     timestamp: Utc::now(),
    /// };
    /// assert_eq!(event.event_type(), "step_started");
    /// ```
    pub fn event_type(&self) -> &'static str {
        match self {
            WorkflowEvent::StepStarted { .. } => Self::STEP_STARTED,
            WorkflowEvent::StepCompleted { .. } => Self::STEP_COMPLETED,
            WorkflowEvent::StepFailed { .. } => Self::STEP_FAILED,
            WorkflowEvent::ApprovalRequired { .. } => Self::APPROVAL_REQUIRED,
            WorkflowEvent::AgentStepTokensUsed { .. } => Self::AGENT_STEP_TOKENS_USED,
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
/// use ironflow_engine::notify::{WorkflowEventBus, WorkflowEvent};
/// use uuid::Uuid;
/// use chrono::Utc;
///
/// let bus = WorkflowEventBus::new();
/// let run_id = Uuid::now_v7();
///
/// let mut rx = bus.subscribe(run_id);
/// bus.publish(run_id, WorkflowEvent::StepStarted {
///     step_name: "build".to_string(),
///     step_index: 0,
///     timestamp: Utc::now(),
/// });
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
    /// use ironflow_engine::notify::{WorkflowEventBus, WorkflowEvent};
    /// use uuid::Uuid;
    /// use chrono::Utc;
    ///
    /// let bus = WorkflowEventBus::new();
    /// let run_id = Uuid::now_v7();
    ///
    /// // No subscriber -- silently dropped.
    /// bus.publish(run_id, WorkflowEvent::StepStarted {
    ///     step_name: "build".to_string(),
    ///     step_index: 0,
    ///     timestamp: Utc::now(),
    /// });
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

    #[tokio::test]
    async fn subscribe_receives_published_events() {
        let bus = WorkflowEventBus::new();
        let run_id = Uuid::now_v7();

        let mut rx = bus.subscribe(run_id);

        let event = WorkflowEvent::StepStarted {
            step_name: "build".to_string(),
            step_index: 0,
            timestamp: Utc::now(),
        };
        bus.publish(run_id, event);

        let received = rx.recv().await.expect("should receive event");
        assert_eq!(received.event_type(), "step_started");
        match received {
            WorkflowEvent::StepStarted {
                step_name,
                step_index,
                ..
            } => {
                assert_eq!(step_name, "build");
                assert_eq!(step_index, 0);
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

        bus.publish(
            unknown_run,
            WorkflowEvent::StepStarted {
                step_name: "build".to_string(),
                step_index: 0,
                timestamp: Utc::now(),
            },
        );
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
            WorkflowEvent::StepStarted {
                step_name: "build".to_string(),
                step_index: 0,
                timestamp: Utc::now(),
            },
            WorkflowEvent::StepCompleted {
                step_name: "deploy".to_string(),
                step_index: 1,
                duration_ms: 5000,
                output_summary: Some("deployed v1.2.3".to_string()),
            },
            WorkflowEvent::StepFailed {
                step_name: "test".to_string(),
                step_index: 2,
                error: "exit code 1".to_string(),
                duration_ms: 3000,
            },
            WorkflowEvent::ApprovalRequired {
                step_name: "prod-gate".to_string(),
                step_index: 3,
                approval_id: Uuid::now_v7(),
            },
            WorkflowEvent::AgentStepTokensUsed {
                step_name: "review".to_string(),
                tokens: 15000,
                cost_usd: Decimal::new(42, 4),
            },
        ];

        for event in &cases {
            let json = serde_json::to_string(event).expect("serialize");
            let back: WorkflowEvent = serde_json::from_str(&json).expect("deserialize");

            assert_eq!(back.event_type(), event.event_type());
            assert!(json.contains(&format!("\"type\":\"{}\"", event.event_type())));
        }
    }

    #[test]
    fn event_type_all_variants() {
        let cases: Vec<(WorkflowEvent, &str)> = vec![
            (
                WorkflowEvent::StepStarted {
                    step_name: "s".to_string(),
                    step_index: 0,
                    timestamp: Utc::now(),
                },
                "step_started",
            ),
            (
                WorkflowEvent::StepCompleted {
                    step_name: "s".to_string(),
                    step_index: 0,
                    duration_ms: 0,
                    output_summary: None,
                },
                "step_completed",
            ),
            (
                WorkflowEvent::StepFailed {
                    step_name: "s".to_string(),
                    step_index: 0,
                    error: "e".to_string(),
                    duration_ms: 0,
                },
                "step_failed",
            ),
            (
                WorkflowEvent::ApprovalRequired {
                    step_name: "s".to_string(),
                    step_index: 0,
                    approval_id: Uuid::now_v7(),
                },
                "approval_required",
            ),
            (
                WorkflowEvent::AgentStepTokensUsed {
                    step_name: "s".to_string(),
                    tokens: 0,
                    cost_usd: Decimal::ZERO,
                },
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

        bus.publish(
            run_id,
            WorkflowEvent::StepStarted {
                step_name: "build".to_string(),
                step_index: 0,
                timestamp: Utc::now(),
            },
        );

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

        bus.publish(
            run_a,
            WorkflowEvent::StepStarted {
                step_name: "only-for-a".to_string(),
                step_index: 0,
                timestamp: Utc::now(),
            },
        );

        let received = rx_a.recv().await.expect("rx_a should receive");
        match received {
            WorkflowEvent::StepStarted { step_name, .. } => {
                assert_eq!(step_name, "only-for-a");
            }
            _ => panic!("expected StepStarted"),
        }

        // rx_b should have nothing -- try_recv returns Empty.
        assert!(rx_b.try_recv().is_err());
    }
}
