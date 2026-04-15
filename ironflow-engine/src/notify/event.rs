//! Domain events emitted throughout the ironflow lifecycle.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use ironflow_store::models::{RunStatus, StepKind};

/// A domain event emitted by the ironflow system.
///
/// Covers the full lifecycle: runs, steps, approvals, and authentication.
/// Subscribers receive these via [`EventPublisher`](super::EventPublisher)
/// and pattern-match on the variants they care about.
///
/// # Examples
///
/// ```
/// use ironflow_engine::notify::Event;
/// use ironflow_store::models::RunStatus;
/// use uuid::Uuid;
///
/// let event = Event::RunStatusChanged {
///     run_id: Uuid::now_v7(),
///     workflow_name: "deploy".to_string(),
///     from: RunStatus::Running,
///     to: RunStatus::Completed,
///     error: None,
///     cost_usd: rust_decimal::Decimal::ZERO,
///     duration_ms: 5000,
///     at: chrono::Utc::now(),
/// };
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    // -- Run lifecycle --
    /// A new run was created (status: Pending).
    RunCreated {
        /// Run identifier.
        run_id: Uuid,
        /// Workflow name.
        workflow_name: String,
        /// When the run was created.
        at: DateTime<Utc>,
    },

    /// A run changed status.
    RunStatusChanged {
        /// Run identifier.
        run_id: Uuid,
        /// Workflow name.
        workflow_name: String,
        /// Previous status.
        from: RunStatus,
        /// New status.
        to: RunStatus,
        /// Error message (when transitioning to Failed).
        error: Option<String>,
        /// Aggregated cost in USD at the time of transition.
        cost_usd: Decimal,
        /// Aggregated duration in milliseconds at the time of transition.
        duration_ms: u64,
        /// When the transition occurred.
        at: DateTime<Utc>,
    },

    /// A run transitioned to [`Failed`](ironflow_store::models::RunStatus::Failed).
    ///
    /// This is a convenience event emitted alongside [`RunStatusChanged`](Event::RunStatusChanged)
    /// when the target status is `Failed`. Subscribe to this instead of
    /// `RUN_STATUS_CHANGED` when you only care about failures.
    RunFailed {
        /// Run identifier.
        run_id: Uuid,
        /// Workflow name.
        workflow_name: String,
        /// Error message.
        error: Option<String>,
        /// Aggregated cost in USD at the time of failure.
        cost_usd: Decimal,
        /// Aggregated duration in milliseconds at the time of failure.
        duration_ms: u64,
        /// When the failure occurred.
        at: DateTime<Utc>,
    },

    // -- Step lifecycle --
    /// A step completed successfully.
    StepCompleted {
        /// Run identifier.
        run_id: Uuid,
        /// Step identifier.
        step_id: Uuid,
        /// Human-readable step name.
        step_name: String,
        /// Step operation kind.
        #[cfg_attr(feature = "openapi", schema(value_type = String))]
        kind: StepKind,
        /// Step duration in milliseconds.
        duration_ms: u64,
        /// Step cost in USD.
        cost_usd: Decimal,
        /// When the step completed.
        at: DateTime<Utc>,
    },

    /// A step failed.
    StepFailed {
        /// Run identifier.
        run_id: Uuid,
        /// Step identifier.
        step_id: Uuid,
        /// Human-readable step name.
        step_name: String,
        /// Step operation kind.
        #[cfg_attr(feature = "openapi", schema(value_type = String))]
        kind: StepKind,
        /// Error message.
        error: String,
        /// When the step failed.
        at: DateTime<Utc>,
    },

    // -- Approval --
    /// A run is waiting for human approval.
    ApprovalRequested {
        /// Run identifier.
        run_id: Uuid,
        /// Approval step identifier.
        step_id: Uuid,
        /// Message displayed to reviewers.
        message: String,
        /// When the approval was requested.
        at: DateTime<Utc>,
    },

    /// A run was approved by a human.
    ApprovalGranted {
        /// Run identifier.
        run_id: Uuid,
        /// User who approved (ID or username).
        approved_by: String,
        /// When the approval was granted.
        at: DateTime<Utc>,
    },

    /// A run was rejected by a human.
    ApprovalRejected {
        /// Run identifier.
        run_id: Uuid,
        /// User who rejected (ID or username).
        rejected_by: String,
        /// When the rejection occurred.
        at: DateTime<Utc>,
    },

    // -- Authentication --
    /// A user signed in.
    UserSignedIn {
        /// User identifier.
        user_id: Uuid,
        /// Username.
        username: String,
        /// When the sign-in occurred.
        at: DateTime<Utc>,
    },

    /// A new user signed up.
    UserSignedUp {
        /// User identifier.
        user_id: Uuid,
        /// Username.
        username: String,
        /// When the sign-up occurred.
        at: DateTime<Utc>,
    },

    /// A user signed out.
    UserSignedOut {
        /// User identifier.
        user_id: Uuid,
        /// When the sign-out occurred.
        at: DateTime<Utc>,
    },
}

impl Event {
    /// Event type constant for [`RunCreated`](Event::RunCreated).
    pub const RUN_CREATED: &'static str = "run_created";
    /// Event type constant for [`RunStatusChanged`](Event::RunStatusChanged).
    pub const RUN_STATUS_CHANGED: &'static str = "run_status_changed";
    /// Event type constant for [`RunFailed`](Event::RunFailed).
    pub const RUN_FAILED: &'static str = "run_failed";
    /// Event type constant for [`StepCompleted`](Event::StepCompleted).
    pub const STEP_COMPLETED: &'static str = "step_completed";
    /// Event type constant for [`StepFailed`](Event::StepFailed).
    pub const STEP_FAILED: &'static str = "step_failed";
    /// Event type constant for [`ApprovalRequested`](Event::ApprovalRequested).
    pub const APPROVAL_REQUESTED: &'static str = "approval_requested";
    /// Event type constant for [`ApprovalGranted`](Event::ApprovalGranted).
    pub const APPROVAL_GRANTED: &'static str = "approval_granted";
    /// Event type constant for [`ApprovalRejected`](Event::ApprovalRejected).
    pub const APPROVAL_REJECTED: &'static str = "approval_rejected";
    /// Event type constant for [`UserSignedIn`](Event::UserSignedIn).
    pub const USER_SIGNED_IN: &'static str = "user_signed_in";
    /// Event type constant for [`UserSignedUp`](Event::UserSignedUp).
    pub const USER_SIGNED_UP: &'static str = "user_signed_up";
    /// Event type constant for [`UserSignedOut`](Event::UserSignedOut).
    pub const USER_SIGNED_OUT: &'static str = "user_signed_out";

    /// All event types. Pass this to
    /// [`EventPublisher::subscribe`](super::EventPublisher::subscribe) to
    /// receive every event.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::notify::{Event, EventPublisher, WebhookSubscriber};
    ///
    /// let mut publisher = EventPublisher::new();
    /// publisher.subscribe(
    ///     WebhookSubscriber::new("https://example.com/all"),
    ///     Event::ALL,
    /// );
    /// ```
    pub const ALL: &'static [&'static str] = &[
        Self::RUN_CREATED,
        Self::RUN_STATUS_CHANGED,
        Self::RUN_FAILED,
        Self::STEP_COMPLETED,
        Self::STEP_FAILED,
        Self::APPROVAL_REQUESTED,
        Self::APPROVAL_GRANTED,
        Self::APPROVAL_REJECTED,
        Self::USER_SIGNED_IN,
        Self::USER_SIGNED_UP,
        Self::USER_SIGNED_OUT,
    ];

    /// Returns the event type as a static string (e.g. `"run_status_changed"`).
    ///
    /// Useful for filtering and logging without deserializing.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::notify::Event;
    /// use uuid::Uuid;
    /// use chrono::Utc;
    ///
    /// let event = Event::UserSignedIn {
    ///     user_id: Uuid::now_v7(),
    ///     username: "alice".to_string(),
    ///     at: Utc::now(),
    /// };
    /// assert_eq!(event.event_type(), "user_signed_in");
    /// ```
    pub fn event_type(&self) -> &'static str {
        match self {
            Event::RunCreated { .. } => Self::RUN_CREATED,
            Event::RunStatusChanged { .. } => Self::RUN_STATUS_CHANGED,
            Event::RunFailed { .. } => Self::RUN_FAILED,
            Event::StepCompleted { .. } => Self::STEP_COMPLETED,
            Event::StepFailed { .. } => Self::STEP_FAILED,
            Event::ApprovalRequested { .. } => Self::APPROVAL_REQUESTED,
            Event::ApprovalGranted { .. } => Self::APPROVAL_GRANTED,
            Event::ApprovalRejected { .. } => Self::APPROVAL_REJECTED,
            Event::UserSignedIn { .. } => Self::USER_SIGNED_IN,
            Event::UserSignedUp { .. } => Self::USER_SIGNED_UP,
            Event::UserSignedOut { .. } => Self::USER_SIGNED_OUT,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_status_changed_serde_roundtrip() {
        let event = Event::RunStatusChanged {
            run_id: Uuid::now_v7(),
            workflow_name: "deploy".to_string(),
            from: RunStatus::Running,
            to: RunStatus::Completed,
            error: None,
            cost_usd: Decimal::new(42, 2),
            duration_ms: 5000,
            at: Utc::now(),
        };

        let json = serde_json::to_string(&event).expect("serialize");
        let back: Event = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(back.event_type(), "run_status_changed");
        assert!(json.contains("\"type\":\"run_status_changed\""));
    }

    #[test]
    fn run_failed_serde_roundtrip() {
        let event = Event::RunFailed {
            run_id: Uuid::now_v7(),
            workflow_name: "deploy".to_string(),
            error: Some("step crashed".to_string()),
            cost_usd: Decimal::new(10, 2),
            duration_ms: 3000,
            at: Utc::now(),
        };

        let json = serde_json::to_string(&event).expect("serialize");
        let back: Event = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(back.event_type(), "run_failed");
        assert!(json.contains("\"type\":\"run_failed\""));
        assert!(json.contains("step crashed"));
    }

    #[test]
    fn user_signed_in_serde_roundtrip() {
        let event = Event::UserSignedIn {
            user_id: Uuid::now_v7(),
            username: "alice".to_string(),
            at: Utc::now(),
        };

        let json = serde_json::to_string(&event).expect("serialize");
        let back: Event = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(back.event_type(), "user_signed_in");
        assert!(json.contains("alice"));
    }

    #[test]
    fn step_failed_serde_roundtrip() {
        let event = Event::StepFailed {
            run_id: Uuid::now_v7(),
            step_id: Uuid::now_v7(),
            step_name: "build".to_string(),
            kind: StepKind::Shell,
            error: "exit code 1".to_string(),
            at: Utc::now(),
        };

        let json = serde_json::to_string(&event).expect("serialize");
        let back: Event = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(back.event_type(), "step_failed");
    }

    #[test]
    fn approval_requested_serde_roundtrip() {
        let event = Event::ApprovalRequested {
            run_id: Uuid::now_v7(),
            step_id: Uuid::now_v7(),
            message: "Deploy to prod?".to_string(),
            at: Utc::now(),
        };

        let json = serde_json::to_string(&event).expect("serialize");
        assert!(json.contains("approval_requested"));
    }

    #[test]
    fn event_type_all_variants() {
        let id = Uuid::now_v7();
        let now = Utc::now();

        let cases: Vec<(Event, &str)> = vec![
            (
                Event::RunCreated {
                    run_id: id,
                    workflow_name: "w".to_string(),
                    at: now,
                },
                "run_created",
            ),
            (
                Event::RunStatusChanged {
                    run_id: id,
                    workflow_name: "w".to_string(),
                    from: RunStatus::Pending,
                    to: RunStatus::Running,
                    error: None,
                    cost_usd: Decimal::ZERO,
                    duration_ms: 0,
                    at: now,
                },
                "run_status_changed",
            ),
            (
                Event::RunFailed {
                    run_id: id,
                    workflow_name: "w".to_string(),
                    error: Some("boom".to_string()),
                    cost_usd: Decimal::ZERO,
                    duration_ms: 0,
                    at: now,
                },
                "run_failed",
            ),
            (
                Event::StepCompleted {
                    run_id: id,
                    step_id: id,
                    step_name: "s".to_string(),
                    kind: StepKind::Shell,
                    duration_ms: 0,
                    cost_usd: Decimal::ZERO,
                    at: now,
                },
                "step_completed",
            ),
            (
                Event::StepFailed {
                    run_id: id,
                    step_id: id,
                    step_name: "s".to_string(),
                    kind: StepKind::Shell,
                    error: "err".to_string(),
                    at: now,
                },
                "step_failed",
            ),
            (
                Event::ApprovalRequested {
                    run_id: id,
                    step_id: id,
                    message: "ok?".to_string(),
                    at: now,
                },
                "approval_requested",
            ),
            (
                Event::ApprovalGranted {
                    run_id: id,
                    approved_by: "alice".to_string(),
                    at: now,
                },
                "approval_granted",
            ),
            (
                Event::ApprovalRejected {
                    run_id: id,
                    rejected_by: "bob".to_string(),
                    at: now,
                },
                "approval_rejected",
            ),
            (
                Event::UserSignedIn {
                    user_id: id,
                    username: "u".to_string(),
                    at: now,
                },
                "user_signed_in",
            ),
            (
                Event::UserSignedUp {
                    user_id: id,
                    username: "u".to_string(),
                    at: now,
                },
                "user_signed_up",
            ),
            (
                Event::UserSignedOut {
                    user_id: id,
                    at: now,
                },
                "user_signed_out",
            ),
        ];

        for (event, expected_type) in cases {
            assert_eq!(event.event_type(), expected_type);
        }
    }
}
