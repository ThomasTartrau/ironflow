//! Domain events emitted throughout the ironflow lifecycle.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub use ironflow_store::entities::LogStream;
use ironflow_store::models::{Assignee, RunStatus, StepKind};

/// Payload of the `Event::RunCreated` event.
///
/// # Examples
///
/// ```
/// use chrono::Utc;
/// use ironflow_engine::notify::RunCreatedEvent;
/// use uuid::Uuid;
///
/// let payload = RunCreatedEvent {
///     run_id: Uuid::now_v7(),
///     workflow_name: "deploy".to_string(),
///     at: Utc::now(),
/// };
/// assert_eq!(payload.workflow_name, "deploy");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RunCreatedEvent {
    /// Run identifier.
    pub run_id: Uuid,
    /// Workflow name.
    pub workflow_name: String,
    /// When the run was created.
    pub at: DateTime<Utc>,
}

/// Payload of the `Event::RunStatusChanged` event.
///
/// # Examples
///
/// ```
/// use std::collections::HashMap;
///
/// use chrono::Utc;
/// use ironflow_engine::notify::RunStatusChangedEvent;
/// use ironflow_store::models::RunStatus;
/// use rust_decimal::Decimal;
/// use uuid::Uuid;
///
/// let payload = RunStatusChangedEvent {
///     run_id: Uuid::now_v7(),
///     workflow_name: "deploy".to_string(),
///     from: RunStatus::Running,
///     to: RunStatus::Completed,
///     error: None,
///     cost_usd: Decimal::ZERO,
///     duration_ms: 5000,
///     labels: HashMap::new(),
///     at: Utc::now(),
/// };
/// assert_eq!(payload.to, RunStatus::Completed);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RunStatusChangedEvent {
    /// Run identifier.
    pub run_id: Uuid,
    /// Workflow name.
    pub workflow_name: String,
    /// Previous status.
    pub from: RunStatus,
    /// New status.
    pub to: RunStatus,
    /// Error message (when transitioning to Failed).
    pub error: Option<String>,
    /// Aggregated cost in USD at the time of transition.
    pub cost_usd: Decimal,
    /// Aggregated duration in milliseconds at the time of transition.
    pub duration_ms: u64,
    /// Labels of the run at the time of the transition.
    #[serde(default)]
    pub labels: HashMap<String, String>,
    /// When the transition occurred.
    pub at: DateTime<Utc>,
}

/// Payload of the `Event::RunFailed` event.
///
/// # Examples
///
/// ```
/// use std::collections::HashMap;
///
/// use chrono::Utc;
/// use ironflow_engine::notify::RunFailedEvent;
/// use rust_decimal::Decimal;
/// use uuid::Uuid;
///
/// let payload = RunFailedEvent {
///     run_id: Uuid::now_v7(),
///     workflow_name: "deploy".to_string(),
///     error: Some("step crashed".to_string()),
///     cost_usd: Decimal::ZERO,
///     duration_ms: 3000,
///     labels: HashMap::new(),
///     at: Utc::now(),
/// };
/// assert_eq!(payload.error.as_deref(), Some("step crashed"));
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RunFailedEvent {
    /// Run identifier.
    pub run_id: Uuid,
    /// Workflow name.
    pub workflow_name: String,
    /// Error message.
    pub error: Option<String>,
    /// Aggregated cost in USD at the time of failure.
    pub cost_usd: Decimal,
    /// Aggregated duration in milliseconds at the time of failure.
    pub duration_ms: u64,
    /// Labels of the run at the time of the failure.
    #[serde(default)]
    pub labels: HashMap<String, String>,
    /// When the failure occurred.
    pub at: DateTime<Utc>,
}

/// Payload of the `Event::RunBudgetExceeded` event.
///
/// # Examples
///
/// ```
/// use chrono::Utc;
/// use ironflow_engine::notify::RunBudgetExceededEvent;
/// use rust_decimal::Decimal;
/// use uuid::Uuid;
///
/// let payload = RunBudgetExceededEvent {
///     run_id: Uuid::now_v7(),
///     workflow_name: "deploy".to_string(),
///     limit_usd: Decimal::new(200, 2),
///     spent_usd: Decimal::new(180, 2),
///     step_budget_usd: Decimal::new(50, 2),
///     at: Utc::now(),
/// };
/// assert_eq!(payload.limit_usd, Decimal::new(200, 2));
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RunBudgetExceededEvent {
    /// Run identifier.
    pub run_id: Uuid,
    /// Workflow name.
    pub workflow_name: String,
    /// The configured cost cap in USD.
    pub limit_usd: Decimal,
    /// Cost already consumed when the cap was reached, in USD.
    pub spent_usd: Decimal,
    /// Declared budget of the refused step, in USD.
    pub step_budget_usd: Decimal,
    /// When the refusal occurred.
    pub at: DateTime<Utc>,
}

/// Payload of the `Event::RetryForced` event.
///
/// # Examples
///
/// ```
/// use chrono::Utc;
/// use ironflow_engine::notify::RetryForcedEvent;
/// use uuid::Uuid;
///
/// let payload = RetryForcedEvent {
///     run_id: Uuid::now_v7(),
///     workflow_name: "deploy".to_string(),
///     original_version: "1".to_string(),
///     current_version: "2".to_string(),
///     at: Utc::now(),
/// };
/// assert_eq!(payload.current_version, "2");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RetryForcedEvent {
    /// The new run created by the forced retry.
    pub run_id: Uuid,
    /// Workflow name.
    pub workflow_name: String,
    /// Version stored on the original run.
    pub original_version: String,
    /// Current version of the handler.
    pub current_version: String,
    /// When the forced retry occurred.
    pub at: DateTime<Utc>,
}

/// Payload of the `Event::StepCompleted` event.
///
/// # Examples
///
/// ```
/// use chrono::Utc;
/// use ironflow_engine::notify::StepCompletedEvent;
/// use ironflow_store::models::StepKind;
/// use rust_decimal::Decimal;
/// use uuid::Uuid;
///
/// let payload = StepCompletedEvent {
///     run_id: Uuid::now_v7(),
///     step_id: Uuid::now_v7(),
///     step_name: "build".to_string(),
///     kind: StepKind::Shell,
///     duration_ms: 1200,
///     cost_usd: Decimal::ZERO,
///     at: Utc::now(),
/// };
/// assert_eq!(payload.step_name, "build");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct StepCompletedEvent {
    /// Run identifier.
    pub run_id: Uuid,
    /// Step identifier.
    pub step_id: Uuid,
    /// Human-readable step name.
    pub step_name: String,
    /// Step operation kind.
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub kind: StepKind,
    /// Step duration in milliseconds.
    pub duration_ms: u64,
    /// Step cost in USD.
    pub cost_usd: Decimal,
    /// When the step completed.
    pub at: DateTime<Utc>,
}

/// Payload of the `Event::StepFailed` event.
///
/// # Examples
///
/// ```
/// use chrono::Utc;
/// use ironflow_engine::notify::StepFailedEvent;
/// use ironflow_store::models::StepKind;
/// use uuid::Uuid;
///
/// let payload = StepFailedEvent {
///     run_id: Uuid::now_v7(),
///     step_id: Uuid::now_v7(),
///     step_name: "build".to_string(),
///     kind: StepKind::Shell,
///     error: "exit code 1".to_string(),
///     at: Utc::now(),
/// };
/// assert_eq!(payload.error, "exit code 1");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct StepFailedEvent {
    /// Run identifier.
    pub run_id: Uuid,
    /// Step identifier.
    pub step_id: Uuid,
    /// Human-readable step name.
    pub step_name: String,
    /// Step operation kind.
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub kind: StepKind,
    /// Error message.
    pub error: String,
    /// When the step failed.
    pub at: DateTime<Utc>,
}

/// Payload of the `Event::ApprovalRequested` event.
///
/// # Examples
///
/// ```
/// use chrono::Utc;
/// use ironflow_engine::notify::ApprovalRequestedEvent;
/// use uuid::Uuid;
///
/// let payload = ApprovalRequestedEvent {
///     run_id: Uuid::now_v7(),
///     step_id: Uuid::now_v7(),
///     message: "Deploy to prod?".to_string(),
///     at: Utc::now(),
/// };
/// assert_eq!(payload.message, "Deploy to prod?");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ApprovalRequestedEvent {
    /// Run identifier.
    pub run_id: Uuid,
    /// Approval step identifier.
    pub step_id: Uuid,
    /// Message displayed to reviewers.
    pub message: String,
    /// When the approval was requested.
    pub at: DateTime<Utc>,
}

/// Payload of the `Event::ApprovalGranted` event.
///
/// # Examples
///
/// ```
/// use chrono::Utc;
/// use ironflow_engine::notify::ApprovalGrantedEvent;
/// use uuid::Uuid;
///
/// let payload = ApprovalGrantedEvent {
///     run_id: Uuid::now_v7(),
///     approved_by: "alice".to_string(),
///     at: Utc::now(),
/// };
/// assert_eq!(payload.approved_by, "alice");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ApprovalGrantedEvent {
    /// Run identifier.
    pub run_id: Uuid,
    /// User who approved (ID or username).
    pub approved_by: String,
    /// When the approval was granted.
    pub at: DateTime<Utc>,
}

/// Payload of the `Event::ApprovalRejected` event.
///
/// # Examples
///
/// ```
/// use chrono::Utc;
/// use ironflow_engine::notify::ApprovalRejectedEvent;
/// use uuid::Uuid;
///
/// let payload = ApprovalRejectedEvent {
///     run_id: Uuid::now_v7(),
///     rejected_by: "bob".to_string(),
///     at: Utc::now(),
/// };
/// assert_eq!(payload.rejected_by, "bob");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ApprovalRejectedEvent {
    /// Run identifier.
    pub run_id: Uuid,
    /// User who rejected (ID or username).
    pub rejected_by: String,
    /// When the rejection occurred.
    pub at: DateTime<Utc>,
}

/// Payload of the `Event::ApprovalEscalated` event.
///
/// Emitted every time an approval gate misses its SLA deadline, including the
/// repeated firings of a bare `Notify`/`Escalate` policy and the final
/// "chain exhausted" notice. The audit log persists it verbatim, so the whole
/// escalation history of a gate is reconstructable from it.
///
/// # Examples
///
/// ```
/// use chrono::Utc;
/// use ironflow_engine::notify::ApprovalEscalatedEvent;
/// use uuid::Uuid;
///
/// let payload = ApprovalEscalatedEvent {
///     run_id: Uuid::now_v7(),
///     step_id: Uuid::now_v7(),
///     step_name: "prod-gate".to_string(),
///     stage: 0,
///     policy: "auto_reject".to_string(),
///     action: "rejected".to_string(),
///     reason: "approval deadline of 3600s expired".to_string(),
///     assignee: None,
///     at: Utc::now(),
/// };
/// assert_eq!(payload.policy, "auto_reject");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ApprovalEscalatedEvent {
    /// Run identifier.
    pub run_id: Uuid,
    /// Approval step identifier.
    pub step_id: Uuid,
    /// Human-readable step name.
    pub step_name: String,
    /// Escalation stage that fired (0-based index into the policy chain).
    pub stage: u32,
    /// Policy applied, e.g. `"auto_reject"`, `"notify"`, `"escalate"`.
    pub policy: String,
    /// What the escalation did, for the audit log.
    pub action: String,
    /// Why it fired, e.g. `"approval deadline of 3600s expired"`.
    pub reason: String,
    /// Assignee after the escalation, when it reassigned the gate.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub assignee: Option<Assignee>,
    /// When the escalation ran.
    pub at: DateTime<Utc>,
}

/// Payload of the `Event::LogLine` event.
///
/// # Examples
///
/// ```
/// use chrono::Utc;
/// use ironflow_engine::notify::{LogLineEvent, LogStream};
/// use uuid::Uuid;
///
/// let payload = LogLineEvent {
///     run_id: Uuid::now_v7(),
///     step_id: Uuid::now_v7(),
///     step_name: "build".to_string(),
///     stream: LogStream::Stdout,
///     line: "Compiling ironflow v0.1.0".to_string(),
///     at: Utc::now(),
/// };
/// assert_eq!(payload.line, "Compiling ironflow v0.1.0");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct LogLineEvent {
    /// Run identifier.
    pub run_id: Uuid,
    /// Step identifier.
    pub step_id: Uuid,
    /// Human-readable step name.
    pub step_name: String,
    /// Output stream.
    pub stream: LogStream,
    /// The log line content.
    pub line: String,
    /// When the line was emitted.
    pub at: DateTime<Utc>,
}

/// Payload of the `Event::UserSignedIn` event.
///
/// # Examples
///
/// ```
/// use chrono::Utc;
/// use ironflow_engine::notify::UserSignedInEvent;
/// use uuid::Uuid;
///
/// let payload = UserSignedInEvent {
///     user_id: Uuid::now_v7(),
///     username: "alice".to_string(),
///     at: Utc::now(),
/// };
/// assert_eq!(payload.username, "alice");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UserSignedInEvent {
    /// User identifier.
    pub user_id: Uuid,
    /// Username.
    pub username: String,
    /// When the sign-in occurred.
    pub at: DateTime<Utc>,
}

/// Payload of the `Event::UserSignedUp` event.
///
/// # Examples
///
/// ```
/// use chrono::Utc;
/// use ironflow_engine::notify::UserSignedUpEvent;
/// use uuid::Uuid;
///
/// let payload = UserSignedUpEvent {
///     user_id: Uuid::now_v7(),
///     username: "alice".to_string(),
///     at: Utc::now(),
/// };
/// assert_eq!(payload.username, "alice");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UserSignedUpEvent {
    /// User identifier.
    pub user_id: Uuid,
    /// Username.
    pub username: String,
    /// When the sign-up occurred.
    pub at: DateTime<Utc>,
}

/// Payload of the `Event::UserSignedOut` event.
///
/// # Examples
///
/// ```
/// use chrono::Utc;
/// use ironflow_engine::notify::UserSignedOutEvent;
/// use uuid::Uuid;
///
/// let user_id = Uuid::now_v7();
/// let payload = UserSignedOutEvent {
///     user_id,
///     at: Utc::now(),
/// };
/// assert_eq!(payload.user_id, user_id);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UserSignedOutEvent {
    /// User identifier.
    pub user_id: Uuid,
    /// When the sign-out occurred.
    pub at: DateTime<Utc>,
}

/// A domain event emitted by the ironflow system.
///
/// Covers the full lifecycle: runs, steps, approvals, and authentication.
/// Subscribers receive these via [`EventPublisher`](super::EventPublisher)
/// and pattern-match on the variants they care about.
///
/// Each variant wraps a dedicated payload struct. The serialized form stays
/// flat: the `type` discriminant sits next to the payload fields, so
/// `{"type":"run_created","run_id":...}` round-trips unchanged.
///
/// # Examples
///
/// ```
/// use std::collections::HashMap;
/// use ironflow_engine::notify::{Event, RunStatusChangedEvent};
/// use ironflow_store::models::RunStatus;
/// use uuid::Uuid;
///
/// let event = Event::RunStatusChanged(RunStatusChangedEvent {
///     run_id: Uuid::now_v7(),
///     workflow_name: "deploy".to_string(),
///     from: RunStatus::Running,
///     to: RunStatus::Completed,
///     error: None,
///     cost_usd: rust_decimal::Decimal::ZERO,
///     duration_ms: 5000,
///     labels: HashMap::new(),
///     at: chrono::Utc::now(),
/// });
/// assert_eq!(event.event_type(), "run_status_changed");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    // -- Run lifecycle --
    /// A new run was created (status: Pending).
    RunCreated(RunCreatedEvent),

    /// A run changed status.
    RunStatusChanged(RunStatusChangedEvent),

    /// A run transitioned to [`Failed`](ironflow_store::models::RunStatus::Failed).
    ///
    /// This is a convenience event emitted alongside [`RunStatusChanged`](Event::RunStatusChanged)
    /// when the target status is `Failed`. Subscribe to this instead of
    /// `RUN_STATUS_CHANGED` when you only care about failures.
    RunFailed(RunFailedEvent),

    /// A run was stopped because it reached its cumulative cost cap.
    ///
    /// Emitted when the engine refuses an agent step that would cross the run's
    /// `max_cost_usd`. The run transitions to
    /// [`Cancelled`](ironflow_store::models::RunStatus::Cancelled) and the step
    /// is never launched, so the reported spend is what the run had already
    /// consumed.
    RunBudgetExceeded(RunBudgetExceededEvent),

    /// A manual retry was forced despite a handler version mismatch.
    ///
    /// Emitted when a caller passes `force=true` on a retry where the
    /// handler version differs from the run's recorded version. This is
    /// an audit event: it means the new code will execute on the old
    /// payload without the handler explicitly declaring compatibility.
    RetryForced(RetryForcedEvent),

    // -- Step lifecycle --
    /// A step completed successfully.
    StepCompleted(StepCompletedEvent),

    /// A step failed.
    StepFailed(StepFailedEvent),

    // -- Approval --
    /// A run is waiting for human approval.
    ApprovalRequested(ApprovalRequestedEvent),

    /// A run was approved by a human.
    ApprovalGranted(ApprovalGrantedEvent),

    /// A run was rejected by a human.
    ApprovalRejected(ApprovalRejectedEvent),

    /// An approval gate missed its SLA deadline and an escalation policy ran.
    ApprovalEscalated(ApprovalEscalatedEvent),

    // -- Log streaming --
    /// A log line emitted during step execution.
    ///
    /// Pushed by the worker in real time so that SSE clients can stream
    /// step output as it happens, without waiting for step completion.
    LogLine(LogLineEvent),

    // -- Authentication --
    /// A user signed in.
    UserSignedIn(UserSignedInEvent),

    /// A new user signed up.
    UserSignedUp(UserSignedUpEvent),

    /// A user signed out.
    UserSignedOut(UserSignedOutEvent),
}

impl Event {
    /// Event type constant for [`RunCreated`](Event::RunCreated).
    pub const RUN_CREATED: &'static str = "run_created";
    /// Event type constant for [`RunStatusChanged`](Event::RunStatusChanged).
    pub const RUN_STATUS_CHANGED: &'static str = "run_status_changed";
    /// Event type constant for [`RunFailed`](Event::RunFailed).
    pub const RUN_FAILED: &'static str = "run_failed";
    /// Event type constant for [`RunBudgetExceeded`](Event::RunBudgetExceeded).
    pub const RUN_BUDGET_EXCEEDED: &'static str = "run_budget_exceeded";
    /// Event type constant for [`RetryForced`](Event::RetryForced).
    pub const RETRY_FORCED: &'static str = "retry_forced";
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
    /// Event type constant for [`ApprovalEscalated`](Event::ApprovalEscalated).
    pub const APPROVAL_ESCALATED: &'static str = "approval_escalated";
    /// Event type constant for [`LogLine`](Event::LogLine).
    pub const LOG_LINE: &'static str = "log_line";
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
        Self::RUN_BUDGET_EXCEEDED,
        Self::STEP_COMPLETED,
        Self::STEP_FAILED,
        Self::APPROVAL_REQUESTED,
        Self::APPROVAL_GRANTED,
        Self::APPROVAL_REJECTED,
        Self::APPROVAL_ESCALATED,
        Self::LOG_LINE,
        Self::USER_SIGNED_IN,
        Self::USER_SIGNED_UP,
        Self::USER_SIGNED_OUT,
        Self::RETRY_FORCED,
    ];

    /// Returns the event type as a static string (e.g. `"run_status_changed"`).
    ///
    /// Useful for filtering and logging without deserializing.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::notify::{Event, UserSignedInEvent};
    /// use uuid::Uuid;
    /// use chrono::Utc;
    ///
    /// let event = Event::UserSignedIn(UserSignedInEvent {
    ///     user_id: Uuid::now_v7(),
    ///     username: "alice".to_string(),
    ///     at: Utc::now(),
    /// });
    /// assert_eq!(event.event_type(), "user_signed_in");
    /// ```
    #[deny(unreachable_patterns)]
    pub fn event_type(&self) -> &'static str {
        match self {
            Event::RunCreated(_) => Self::RUN_CREATED,
            Event::RunStatusChanged(_) => Self::RUN_STATUS_CHANGED,
            Event::RunFailed(_) => Self::RUN_FAILED,
            Event::RunBudgetExceeded(_) => Self::RUN_BUDGET_EXCEEDED,
            Event::RetryForced(_) => Self::RETRY_FORCED,
            Event::StepCompleted(_) => Self::STEP_COMPLETED,
            Event::StepFailed(_) => Self::STEP_FAILED,
            Event::ApprovalRequested(_) => Self::APPROVAL_REQUESTED,
            Event::ApprovalGranted(_) => Self::APPROVAL_GRANTED,
            Event::ApprovalRejected(_) => Self::APPROVAL_REJECTED,
            Event::ApprovalEscalated(_) => Self::APPROVAL_ESCALATED,
            Event::LogLine(_) => Self::LOG_LINE,
            Event::UserSignedIn(_) => Self::USER_SIGNED_IN,
            Event::UserSignedUp(_) => Self::USER_SIGNED_UP,
            Event::UserSignedOut(_) => Self::USER_SIGNED_OUT,
        }
    }

    /// Returns the run this event belongs to, if any.
    ///
    /// Auth events ([`UserSignedIn`](Event::UserSignedIn),
    /// [`UserSignedUp`](Event::UserSignedUp),
    /// [`UserSignedOut`](Event::UserSignedOut)) are not tied to a run and
    /// return `None`.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::notify::{Event, RunCreatedEvent};
    /// use uuid::Uuid;
    /// use chrono::Utc;
    ///
    /// let run_id = Uuid::now_v7();
    /// let event = Event::RunCreated(RunCreatedEvent {
    ///     run_id,
    ///     workflow_name: "deploy".to_string(),
    ///     at: Utc::now(),
    /// });
    /// assert_eq!(event.run_id(), Some(run_id));
    /// ```
    #[deny(unreachable_patterns)]
    pub fn run_id(&self) -> Option<Uuid> {
        match self {
            Event::RunCreated(e) => Some(e.run_id),
            Event::RunStatusChanged(e) => Some(e.run_id),
            Event::RunFailed(e) => Some(e.run_id),
            Event::RunBudgetExceeded(e) => Some(e.run_id),
            Event::RetryForced(e) => Some(e.run_id),
            Event::StepCompleted(e) => Some(e.run_id),
            Event::StepFailed(e) => Some(e.run_id),
            Event::ApprovalRequested(e) => Some(e.run_id),
            Event::ApprovalGranted(e) => Some(e.run_id),
            Event::ApprovalRejected(e) => Some(e.run_id),
            Event::ApprovalEscalated(e) => Some(e.run_id),
            Event::LogLine(e) => Some(e.run_id),
            Event::UserSignedIn(_) | Event::UserSignedUp(_) | Event::UserSignedOut(_) => None,
        }
    }

    /// Returns the step this event belongs to, if any.
    ///
    /// Only [`StepCompleted`](Event::StepCompleted),
    /// [`StepFailed`](Event::StepFailed),
    /// [`ApprovalRequested`](Event::ApprovalRequested) and
    /// [`ApprovalEscalated`](Event::ApprovalEscalated) carry a step
    /// identifier; every other variant returns `None`.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::notify::{Event, StepFailedEvent};
    /// use ironflow_store::models::StepKind;
    /// use uuid::Uuid;
    /// use chrono::Utc;
    ///
    /// let step_id = Uuid::now_v7();
    /// let event = Event::StepFailed(StepFailedEvent {
    ///     run_id: Uuid::now_v7(),
    ///     step_id,
    ///     step_name: "build".to_string(),
    ///     kind: StepKind::Shell,
    ///     error: "exit code 1".to_string(),
    ///     at: Utc::now(),
    /// });
    /// assert_eq!(event.step_id(), Some(step_id));
    /// ```
    #[deny(unreachable_patterns)]
    pub fn step_id(&self) -> Option<Uuid> {
        match self {
            Event::StepCompleted(e) => Some(e.step_id),
            Event::StepFailed(e) => Some(e.step_id),
            Event::ApprovalRequested(e) => Some(e.step_id),
            Event::ApprovalEscalated(e) => Some(e.step_id),
            Event::RunCreated(_)
            | Event::RunStatusChanged(_)
            | Event::RunFailed(_)
            | Event::RunBudgetExceeded(_)
            | Event::RetryForced(_)
            | Event::ApprovalGranted(_)
            | Event::ApprovalRejected(_)
            | Event::LogLine(_)
            | Event::UserSignedIn(_)
            | Event::UserSignedUp(_)
            | Event::UserSignedOut(_) => None,
        }
    }

    /// Returns the user this event belongs to, if any.
    ///
    /// Only the auth events ([`UserSignedIn`](Event::UserSignedIn),
    /// [`UserSignedUp`](Event::UserSignedUp),
    /// [`UserSignedOut`](Event::UserSignedOut)) carry a user identifier;
    /// every other variant returns `None`.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::notify::{Event, UserSignedInEvent};
    /// use uuid::Uuid;
    /// use chrono::Utc;
    ///
    /// let user_id = Uuid::now_v7();
    /// let event = Event::UserSignedIn(UserSignedInEvent {
    ///     user_id,
    ///     username: "alice".to_string(),
    ///     at: Utc::now(),
    /// });
    /// assert_eq!(event.user_id(), Some(user_id));
    /// ```
    #[deny(unreachable_patterns)]
    pub fn user_id(&self) -> Option<Uuid> {
        match self {
            Event::UserSignedIn(e) => Some(e.user_id),
            Event::UserSignedUp(e) => Some(e.user_id),
            Event::UserSignedOut(e) => Some(e.user_id),
            Event::RunCreated(_)
            | Event::RunStatusChanged(_)
            | Event::RunFailed(_)
            | Event::RunBudgetExceeded(_)
            | Event::RetryForced(_)
            | Event::StepCompleted(_)
            | Event::StepFailed(_)
            | Event::ApprovalRequested(_)
            | Event::ApprovalGranted(_)
            | Event::ApprovalRejected(_)
            | Event::ApprovalEscalated(_)
            | Event::LogLine(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_status_changed_serde_roundtrip() {
        let event = Event::RunStatusChanged(RunStatusChangedEvent {
            run_id: Uuid::now_v7(),
            workflow_name: "deploy".to_string(),
            from: RunStatus::Running,
            to: RunStatus::Completed,
            error: None,
            cost_usd: Decimal::new(42, 2),
            duration_ms: 5000,
            labels: HashMap::new(),
            at: Utc::now(),
        });

        let json = serde_json::to_string(&event).expect("serialize");
        let back: Event = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(back.event_type(), "run_status_changed");
        assert!(json.contains("\"type\":\"run_status_changed\""));
    }

    #[test]
    fn run_failed_serde_roundtrip() {
        let event = Event::RunFailed(RunFailedEvent {
            run_id: Uuid::now_v7(),
            workflow_name: "deploy".to_string(),
            error: Some("step crashed".to_string()),
            cost_usd: Decimal::new(10, 2),
            duration_ms: 3000,
            labels: HashMap::new(),
            at: Utc::now(),
        });

        let json = serde_json::to_string(&event).expect("serialize");
        let back: Event = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(back.event_type(), "run_failed");
        assert!(json.contains("\"type\":\"run_failed\""));
        assert!(json.contains("step crashed"));
    }

    #[test]
    fn run_budget_exceeded_serde_roundtrip() {
        let event = Event::RunBudgetExceeded(RunBudgetExceededEvent {
            run_id: Uuid::now_v7(),
            workflow_name: "deploy".to_string(),
            limit_usd: Decimal::new(200, 2),
            spent_usd: Decimal::new(180, 2),
            step_budget_usd: Decimal::new(50, 2),
            at: Utc::now(),
        });

        let json = serde_json::to_string(&event).expect("serialize");
        let back: Event = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(back.event_type(), "run_budget_exceeded");
        assert!(json.contains("\"type\":\"run_budget_exceeded\""));
        assert!(json.contains("limit_usd"));
        assert!(json.contains("step_budget_usd"));
    }

    #[test]
    fn all_contains_run_budget_exceeded() {
        assert!(Event::ALL.contains(&Event::RUN_BUDGET_EXCEEDED));
    }

    #[test]
    fn user_signed_in_serde_roundtrip() {
        let event = Event::UserSignedIn(UserSignedInEvent {
            user_id: Uuid::now_v7(),
            username: "alice".to_string(),
            at: Utc::now(),
        });

        let json = serde_json::to_string(&event).expect("serialize");
        let back: Event = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(back.event_type(), "user_signed_in");
        assert!(json.contains("alice"));
    }

    #[test]
    fn step_failed_serde_roundtrip() {
        let event = Event::StepFailed(StepFailedEvent {
            run_id: Uuid::now_v7(),
            step_id: Uuid::now_v7(),
            step_name: "build".to_string(),
            kind: StepKind::Shell,
            error: "exit code 1".to_string(),
            at: Utc::now(),
        });

        let json = serde_json::to_string(&event).expect("serialize");
        let back: Event = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(back.event_type(), "step_failed");
    }

    #[test]
    fn approval_requested_serde_roundtrip() {
        let event = Event::ApprovalRequested(ApprovalRequestedEvent {
            run_id: Uuid::now_v7(),
            step_id: Uuid::now_v7(),
            message: "Deploy to prod?".to_string(),
            at: Utc::now(),
        });

        let json = serde_json::to_string(&event).expect("serialize");
        assert!(json.contains("approval_requested"));
    }

    #[test]
    fn log_line_serde_roundtrip() {
        let event = Event::LogLine(LogLineEvent {
            run_id: Uuid::now_v7(),
            step_id: Uuid::now_v7(),
            step_name: "build".to_string(),
            stream: LogStream::Stdout,
            line: "Compiling ironflow v0.1.0".to_string(),
            at: Utc::now(),
        });

        let json = serde_json::to_string(&event).expect("serialize");
        let back: Event = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(back.event_type(), "log_line");
        assert!(json.contains("\"type\":\"log_line\""));
        assert!(json.contains("Compiling ironflow"));
    }

    /// The pre-refactor wire format used flat inline-struct variants. Newtype
    /// variants over named-field payloads produce and accept the exact same
    /// JSON, so audit rows and in-flight payloads written before the refactor
    /// still deserialize. No data migration is required.
    #[test]
    fn legacy_flat_json_deserializes_into_typed_payload() {
        let run_id: Uuid = "01890000-0000-7000-8000-000000000000"
            .parse()
            .expect("valid uuid");

        let raw = r#"{"type":"run_created","run_id":"01890000-0000-7000-8000-000000000000","workflow_name":"deploy","at":"2026-01-01T00:00:00Z"}"#;
        let event: Event = serde_json::from_str(raw).expect("legacy payload must deserialize");
        match event {
            Event::RunCreated(e) => {
                assert_eq!(e.run_id, run_id);
                assert_eq!(e.workflow_name, "deploy");
            }
            other => panic!("expected RunCreated, got {other:?}"),
        }

        let raw = r#"{"type":"run_status_changed","run_id":"01890000-0000-7000-8000-000000000000","workflow_name":"deploy","from":"running","to":"completed","error":null,"cost_usd":0.5,"duration_ms":5000,"labels":{"env":"prod"},"at":"2026-01-01T00:00:00Z"}"#;
        let event: Event = serde_json::from_str(raw).expect("legacy payload must deserialize");
        match event {
            Event::RunStatusChanged(e) => {
                assert_eq!(e.from, RunStatus::Running);
                assert_eq!(e.to, RunStatus::Completed);
                assert_eq!(e.cost_usd, Decimal::new(5, 1));
                assert_eq!(e.duration_ms, 5000);
                assert_eq!(e.labels.get("env").map(String::as_str), Some("prod"));
            }
            other => panic!("expected RunStatusChanged, got {other:?}"),
        }

        // `labels` predates no payload: omitting it must still work via `#[serde(default)]`.
        let raw = r#"{"type":"run_status_changed","run_id":"01890000-0000-7000-8000-000000000000","workflow_name":"deploy","from":"running","to":"failed","error":"boom","cost_usd":0,"duration_ms":0,"at":"2026-01-01T00:00:00Z"}"#;
        let event: Event = serde_json::from_str(raw).expect("missing labels must default");
        match event {
            Event::RunStatusChanged(e) => {
                assert!(e.labels.is_empty());
                assert_eq!(e.error.as_deref(), Some("boom"));
            }
            other => panic!("expected RunStatusChanged, got {other:?}"),
        }

        let raw = r#"{"type":"run_failed","run_id":"01890000-0000-7000-8000-000000000000","workflow_name":"deploy","error":"boom","cost_usd":0.25,"duration_ms":3000,"at":"2026-01-01T00:00:00Z"}"#;
        let event: Event = serde_json::from_str(raw).expect("legacy payload must deserialize");
        match event {
            Event::RunFailed(e) => {
                assert_eq!(e.error.as_deref(), Some("boom"));
                assert!(e.labels.is_empty());
            }
            other => panic!("expected RunFailed, got {other:?}"),
        }

        let raw = r#"{"type":"step_failed","run_id":"01890000-0000-7000-8000-000000000000","step_id":"01890000-0000-7000-8000-000000000001","step_name":"build","kind":"shell","error":"exit code 1","at":"2026-01-01T00:00:00Z"}"#;
        let event: Event = serde_json::from_str(raw).expect("legacy payload must deserialize");
        match event {
            Event::StepFailed(e) => {
                assert_eq!(e.kind, StepKind::Shell);
                assert_eq!(e.error, "exit code 1");
            }
            other => panic!("expected StepFailed, got {other:?}"),
        }

        let raw = r#"{"type":"log_line","run_id":"01890000-0000-7000-8000-000000000000","step_id":"01890000-0000-7000-8000-000000000001","step_name":"build","stream":"stdout","line":"hello","at":"2026-01-01T00:00:00Z"}"#;
        let event: Event = serde_json::from_str(raw).expect("legacy payload must deserialize");
        match event {
            Event::LogLine(e) => {
                assert_eq!(e.stream, LogStream::Stdout);
                assert_eq!(e.line, "hello");
            }
            other => panic!("expected LogLine, got {other:?}"),
        }

        let raw = r#"{"type":"user_signed_in","user_id":"01890000-0000-7000-8000-000000000000","username":"alice","at":"2026-01-01T00:00:00Z"}"#;
        let event: Event = serde_json::from_str(raw).expect("legacy payload must deserialize");
        match event {
            Event::UserSignedIn(e) => assert_eq!(e.username, "alice"),
            other => panic!("expected UserSignedIn, got {other:?}"),
        }
    }

    /// Guards the internally-tagged representation: payload fields must stay
    /// siblings of `type`, never nested under a variant key.
    #[test]
    fn serialized_event_is_flat_with_type_tag() {
        let run_id = Uuid::now_v7();
        let event = Event::RunCreated(RunCreatedEvent {
            run_id,
            workflow_name: "deploy".to_string(),
            at: Utc::now(),
        });

        let value: serde_json::Value = serde_json::to_value(&event).expect("serialize");
        let object = value.as_object().expect("event serializes to an object");

        assert_eq!(
            object.get("type").and_then(|v| v.as_str()),
            Some("run_created")
        );
        assert_eq!(
            object.get("workflow_name").and_then(|v| v.as_str()),
            Some("deploy")
        );
        assert_eq!(
            object.get("run_id").and_then(|v| v.as_str()),
            Some(run_id.to_string().as_str())
        );
        assert!(object.contains_key("at"));
        assert_eq!(object.len(), 4, "no nesting: {object:?}");
        assert!(!object.contains_key("RunCreated"));
    }

    #[test]
    fn run_id_returns_some_for_run_events() {
        let run_id = Uuid::now_v7();
        let now = Utc::now();

        let events = vec![
            Event::RunCreated(RunCreatedEvent {
                run_id,
                workflow_name: "w".to_string(),
                at: now,
            }),
            Event::RunStatusChanged(RunStatusChangedEvent {
                run_id,
                workflow_name: "w".to_string(),
                from: RunStatus::Pending,
                to: RunStatus::Running,
                error: None,
                cost_usd: Decimal::ZERO,
                duration_ms: 0,
                labels: HashMap::new(),
                at: now,
            }),
            Event::RunFailed(RunFailedEvent {
                run_id,
                workflow_name: "w".to_string(),
                error: None,
                cost_usd: Decimal::ZERO,
                duration_ms: 0,
                labels: HashMap::new(),
                at: now,
            }),
            Event::RunBudgetExceeded(RunBudgetExceededEvent {
                run_id,
                workflow_name: "w".to_string(),
                limit_usd: Decimal::ZERO,
                spent_usd: Decimal::ZERO,
                step_budget_usd: Decimal::ZERO,
                at: now,
            }),
            Event::RetryForced(RetryForcedEvent {
                run_id,
                workflow_name: "w".to_string(),
                original_version: "1".to_string(),
                current_version: "2".to_string(),
                at: now,
            }),
            Event::StepCompleted(StepCompletedEvent {
                run_id,
                step_id: Uuid::now_v7(),
                step_name: "s".to_string(),
                kind: StepKind::Shell,
                duration_ms: 0,
                cost_usd: Decimal::ZERO,
                at: now,
            }),
            Event::StepFailed(StepFailedEvent {
                run_id,
                step_id: Uuid::now_v7(),
                step_name: "s".to_string(),
                kind: StepKind::Shell,
                error: "e".to_string(),
                at: now,
            }),
            Event::ApprovalRequested(ApprovalRequestedEvent {
                run_id,
                step_id: Uuid::now_v7(),
                message: "ok?".to_string(),
                at: now,
            }),
            Event::ApprovalGranted(ApprovalGrantedEvent {
                run_id,
                approved_by: "alice".to_string(),
                at: now,
            }),
            Event::ApprovalRejected(ApprovalRejectedEvent {
                run_id,
                rejected_by: "bob".to_string(),
                at: now,
            }),
            Event::LogLine(LogLineEvent {
                run_id,
                step_id: Uuid::now_v7(),
                step_name: "s".to_string(),
                stream: LogStream::Stdout,
                line: "l".to_string(),
                at: now,
            }),
        ];

        for event in &events {
            assert_eq!(
                event.run_id(),
                Some(run_id),
                "{} should carry a run_id",
                event.event_type()
            );
        }
    }

    #[test]
    fn run_id_returns_none_for_auth_events() {
        let user_id = Uuid::now_v7();
        let now = Utc::now();

        let events = vec![
            Event::UserSignedIn(UserSignedInEvent {
                user_id,
                username: "alice".to_string(),
                at: now,
            }),
            Event::UserSignedUp(UserSignedUpEvent {
                user_id,
                username: "alice".to_string(),
                at: now,
            }),
            Event::UserSignedOut(UserSignedOutEvent { user_id, at: now }),
        ];

        for event in &events {
            assert_eq!(event.run_id(), None, "{} has no run", event.event_type());
        }
    }

    #[test]
    fn step_id_returns_some_only_for_step_events() {
        let step_id = Uuid::now_v7();
        let run_id = Uuid::now_v7();
        let now = Utc::now();

        let with_step = vec![
            Event::StepCompleted(StepCompletedEvent {
                run_id,
                step_id,
                step_name: "s".to_string(),
                kind: StepKind::Shell,
                duration_ms: 0,
                cost_usd: Decimal::ZERO,
                at: now,
            }),
            Event::StepFailed(StepFailedEvent {
                run_id,
                step_id,
                step_name: "s".to_string(),
                kind: StepKind::Shell,
                error: "e".to_string(),
                at: now,
            }),
            Event::ApprovalRequested(ApprovalRequestedEvent {
                run_id,
                step_id,
                message: "ok?".to_string(),
                at: now,
            }),
        ];

        for event in &with_step {
            assert_eq!(
                event.step_id(),
                Some(step_id),
                "{} should carry a step_id",
                event.event_type()
            );
        }

        let without_step = vec![
            Event::RunCreated(RunCreatedEvent {
                run_id,
                workflow_name: "w".to_string(),
                at: now,
            }),
            Event::ApprovalGranted(ApprovalGrantedEvent {
                run_id,
                approved_by: "alice".to_string(),
                at: now,
            }),
            // LogLine carries a step_id field but is reported as a run-level
            // stream event, matching the pre-refactor behaviour.
            Event::LogLine(LogLineEvent {
                run_id,
                step_id,
                step_name: "s".to_string(),
                stream: LogStream::Stdout,
                line: "l".to_string(),
                at: now,
            }),
            Event::UserSignedOut(UserSignedOutEvent {
                user_id: Uuid::now_v7(),
                at: now,
            }),
        ];

        for event in &without_step {
            assert_eq!(
                event.step_id(),
                None,
                "{} should not carry a step_id",
                event.event_type()
            );
        }
    }

    #[test]
    fn user_id_returns_some_only_for_auth_events() {
        let user_id = Uuid::now_v7();
        let run_id = Uuid::now_v7();
        let now = Utc::now();

        let auth = vec![
            Event::UserSignedIn(UserSignedInEvent {
                user_id,
                username: "alice".to_string(),
                at: now,
            }),
            Event::UserSignedUp(UserSignedUpEvent {
                user_id,
                username: "alice".to_string(),
                at: now,
            }),
            Event::UserSignedOut(UserSignedOutEvent { user_id, at: now }),
        ];

        for event in &auth {
            assert_eq!(
                event.user_id(),
                Some(user_id),
                "{} should carry a user_id",
                event.event_type()
            );
        }

        let non_auth = vec![
            Event::RunCreated(RunCreatedEvent {
                run_id,
                workflow_name: "w".to_string(),
                at: now,
            }),
            Event::StepFailed(StepFailedEvent {
                run_id,
                step_id: Uuid::now_v7(),
                step_name: "s".to_string(),
                kind: StepKind::Shell,
                error: "e".to_string(),
                at: now,
            }),
        ];

        for event in &non_auth {
            assert_eq!(
                event.user_id(),
                None,
                "{} should not carry a user_id",
                event.event_type()
            );
        }
    }

    #[test]
    fn event_type_all_variants() {
        let id = Uuid::now_v7();
        let now = Utc::now();

        let cases: Vec<(Event, &str)> = vec![
            (
                Event::RunCreated(RunCreatedEvent {
                    run_id: id,
                    workflow_name: "w".to_string(),
                    at: now,
                }),
                "run_created",
            ),
            (
                Event::RunStatusChanged(RunStatusChangedEvent {
                    run_id: id,
                    workflow_name: "w".to_string(),
                    from: RunStatus::Pending,
                    to: RunStatus::Running,
                    error: None,
                    cost_usd: Decimal::ZERO,
                    duration_ms: 0,
                    labels: HashMap::new(),
                    at: now,
                }),
                "run_status_changed",
            ),
            (
                Event::RunFailed(RunFailedEvent {
                    run_id: id,
                    workflow_name: "w".to_string(),
                    error: Some("boom".to_string()),
                    cost_usd: Decimal::ZERO,
                    duration_ms: 0,
                    labels: HashMap::new(),
                    at: now,
                }),
                "run_failed",
            ),
            (
                Event::RunBudgetExceeded(RunBudgetExceededEvent {
                    run_id: id,
                    workflow_name: "w".to_string(),
                    limit_usd: Decimal::new(200, 2),
                    spent_usd: Decimal::new(180, 2),
                    step_budget_usd: Decimal::new(50, 2),
                    at: now,
                }),
                "run_budget_exceeded",
            ),
            (
                Event::RetryForced(RetryForcedEvent {
                    run_id: id,
                    workflow_name: "w".to_string(),
                    original_version: "1".to_string(),
                    current_version: "2".to_string(),
                    at: now,
                }),
                "retry_forced",
            ),
            (
                Event::StepCompleted(StepCompletedEvent {
                    run_id: id,
                    step_id: id,
                    step_name: "s".to_string(),
                    kind: StepKind::Shell,
                    duration_ms: 0,
                    cost_usd: Decimal::ZERO,
                    at: now,
                }),
                "step_completed",
            ),
            (
                Event::StepFailed(StepFailedEvent {
                    run_id: id,
                    step_id: id,
                    step_name: "s".to_string(),
                    kind: StepKind::Shell,
                    error: "err".to_string(),
                    at: now,
                }),
                "step_failed",
            ),
            (
                Event::ApprovalRequested(ApprovalRequestedEvent {
                    run_id: id,
                    step_id: id,
                    message: "ok?".to_string(),
                    at: now,
                }),
                "approval_requested",
            ),
            (
                Event::ApprovalGranted(ApprovalGrantedEvent {
                    run_id: id,
                    approved_by: "alice".to_string(),
                    at: now,
                }),
                "approval_granted",
            ),
            (
                Event::ApprovalRejected(ApprovalRejectedEvent {
                    run_id: id,
                    rejected_by: "bob".to_string(),
                    at: now,
                }),
                "approval_rejected",
            ),
            (
                Event::ApprovalEscalated(ApprovalEscalatedEvent {
                    run_id: id,
                    step_id: id,
                    step_name: "prod-gate".to_string(),
                    stage: 0,
                    policy: "auto_reject".to_string(),
                    action: "rejected".to_string(),
                    reason: "approval deadline of 3600s expired".to_string(),
                    assignee: None,
                    at: now,
                }),
                "approval_escalated",
            ),
            (
                Event::LogLine(LogLineEvent {
                    run_id: id,
                    step_id: id,
                    step_name: "build".to_string(),
                    stream: LogStream::Stdout,
                    line: "Compiling ironflow v0.1.0".to_string(),
                    at: now,
                }),
                "log_line",
            ),
            (
                Event::UserSignedIn(UserSignedInEvent {
                    user_id: id,
                    username: "u".to_string(),
                    at: now,
                }),
                "user_signed_in",
            ),
            (
                Event::UserSignedUp(UserSignedUpEvent {
                    user_id: id,
                    username: "u".to_string(),
                    at: now,
                }),
                "user_signed_up",
            ),
            (
                Event::UserSignedOut(UserSignedOutEvent {
                    user_id: id,
                    at: now,
                }),
                "user_signed_out",
            ),
        ];

        assert_eq!(
            cases.len(),
            Event::ALL.len(),
            "every variant must be covered"
        );

        for (event, expected_type) in cases {
            assert_eq!(event.event_type(), expected_type);
        }
    }

    #[test]
    fn approval_escalated_serde_roundtrip() {
        let run_id = Uuid::now_v7();
        let step_id = Uuid::now_v7();
        let event = Event::ApprovalEscalated(ApprovalEscalatedEvent {
            run_id,
            step_id,
            step_name: "prod-gate".to_string(),
            stage: 1,
            policy: "escalate".to_string(),
            action: "reassigned to sre-oncall".to_string(),
            reason: "approval deadline of 3600s expired".to_string(),
            assignee: Some(Assignee::group("sre-oncall")),
            at: Utc::now(),
        });

        let json = serde_json::to_string(&event).expect("serialize");
        assert!(
            json.contains("\"type\":\"approval_escalated\""),
            "got {json}"
        );

        let back: Event = serde_json::from_str(&json).expect("deserialize");
        let Event::ApprovalEscalated(payload) = back else {
            panic!("expected an approval_escalated event");
        };
        assert_eq!(payload.run_id, run_id);
        assert_eq!(payload.stage, 1);
        assert_eq!(payload.assignee, Some(Assignee::group("sre-oncall")));
    }

    #[test]
    fn approval_escalated_carries_run_and_step_ids() {
        let run_id = Uuid::now_v7();
        let step_id = Uuid::now_v7();
        let event = Event::ApprovalEscalated(ApprovalEscalatedEvent {
            run_id,
            step_id,
            step_name: "prod-gate".to_string(),
            stage: 0,
            policy: "notify".to_string(),
            action: "notified 1 target".to_string(),
            reason: "approval deadline of 60s expired".to_string(),
            assignee: None,
            at: Utc::now(),
        });

        assert_eq!(event.run_id(), Some(run_id));
        assert_eq!(event.step_id(), Some(step_id));
        assert_eq!(event.user_id(), None);
    }
}
