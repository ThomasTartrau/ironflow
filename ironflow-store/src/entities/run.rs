//! [`Run`] entity and related request/update types.

use std::collections::HashMap;

use chrono::{DateTime, TimeDelta, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use super::{FsmState, RunStatus, TriggerKind};

/// A workflow execution record.
///
/// Represents a single invocation of a workflow, tracking its status through
/// the [`RunStatus`] FSM (SQL-side via [`lib_fsm`](crate::postgres::helpers::lib_fsm)),
/// aggregated metrics, and timestamps.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::Run;
///
/// // Runs are created by RunStore::create_run, not directly.
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Run {
    /// Unique identifier (UUIDv7, sortable by creation time).
    pub id: Uuid,
    /// Name of the workflow that was executed.
    pub workflow_name: String,
    /// Current FSM status — embeds state + state_machine_id for SQL-side transitions.
    pub status: FsmState<RunStatus>,
    /// How this run was triggered.
    pub trigger: TriggerKind,
    /// Trigger-specific payload (e.g. webhook body).
    pub payload: Value,
    /// Error message if the run failed.
    pub error: Option<String>,
    /// Number of times this run has been retried.
    pub retry_count: u32,
    /// Maximum number of retries allowed.
    pub max_retries: u32,
    /// Aggregated cost across all agent steps, in USD.
    pub cost_usd: Decimal,
    /// Aggregated wall-clock duration across all steps, in milliseconds.
    pub duration_ms: u64,
    /// When the run was created (enqueued).
    pub created_at: DateTime<Utc>,
    /// When the run record was last updated.
    pub updated_at: DateTime<Utc>,
    /// When execution started (transitioned to Running).
    pub started_at: Option<DateTime<Utc>>,
    /// When execution finished (transitioned to a terminal state).
    pub completed_at: Option<DateTime<Utc>>,
    /// Version of the handler that created this run.
    pub handler_version: Option<String>,
    /// User-defined key-value labels for categorization and filtering.
    #[serde(default)]
    pub labels: HashMap<String, String>,
    /// When the run should start executing. `None` means immediately.
    #[serde(default)]
    pub scheduled_at: Option<DateTime<Utc>>,
    /// Client-supplied idempotency key that produced this run, if any.
    ///
    /// See [`IDEMPOTENCY_WINDOW`] for how long a key stays bound to its run.
    #[serde(default)]
    pub idempotency_key: Option<String>,
}

/// How long a client-supplied idempotency key stays bound to its run.
///
/// Past this window a replayed key no longer resolves to the original run:
/// the key is released and a fresh run is created.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::IDEMPOTENCY_WINDOW;
///
/// assert_eq!(IDEMPOTENCY_WINDOW.num_hours(), 24);
/// ```
pub const IDEMPOTENCY_WINDOW: TimeDelta = TimeDelta::hours(24);

/// Maximum accepted length of an idempotency key, in bytes.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::MAX_IDEMPOTENCY_KEY_LEN;
///
/// assert_eq!(MAX_IDEMPOTENCY_KEY_LEN, 255);
/// ```
pub const MAX_IDEMPOTENCY_KEY_LEN: usize = 255;

/// Outcome of [`RunStore::create_run`](crate::store::RunStore::create_run).
///
/// A request carrying an idempotency key already bound to a live run does not
/// insert anything: the store returns the original run as [`RunCreation::Existing`].
///
/// # Examples
///
/// ```
/// use std::collections::HashMap;
/// use ironflow_store::entities::{NewRun, RunCreation, TriggerKind};
/// use ironflow_store::memory::InMemoryStore;
/// use ironflow_store::store::RunStore;
/// use serde_json::json;
///
/// # async fn example() -> Result<(), ironflow_store::error::StoreError> {
/// let store = InMemoryStore::new();
/// let req = NewRun {
///     workflow_name: "deploy".to_string(),
///     trigger: TriggerKind::Manual,
///     payload: json!({}),
///     max_retries: 3,
///     handler_version: None,
///     labels: HashMap::new(),
///     scheduled_at: None,
///     idempotency_key: Some("deploy-2026-07-26".to_string()),
/// };
///
/// assert!(store.create_run(req.clone()).await?.is_created());
/// assert!(!store.create_run(req).await?.is_created());
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RunCreation {
    /// A new run was inserted.
    Created(Run),
    /// The idempotency key already resolved to this run; nothing was inserted.
    Existing(Run),
}

impl RunCreation {
    /// Return the run, discarding whether it was created or replayed.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use ironflow_store::entities::RunCreation;
    /// # fn example(creation: RunCreation) {
    /// let run = creation.into_run();
    /// # }
    /// ```
    pub fn into_run(self) -> Run {
        match self {
            RunCreation::Created(run) | RunCreation::Existing(run) => run,
        }
    }

    /// Borrow the run, discarding whether it was created or replayed.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use ironflow_store::entities::RunCreation;
    /// # fn example(creation: &RunCreation) {
    /// let id = creation.run().id;
    /// # }
    /// ```
    pub fn run(&self) -> &Run {
        match self {
            RunCreation::Created(run) | RunCreation::Existing(run) => run,
        }
    }

    /// Whether a new run was actually inserted.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use ironflow_store::entities::RunCreation;
    /// # fn example(creation: &RunCreation) {
    /// if creation.is_created() {
    ///     // publish a RunCreated event
    /// }
    /// # }
    /// ```
    pub fn is_created(&self) -> bool {
        matches!(self, RunCreation::Created(_))
    }
}

/// Request to create a new run.
///
/// # Examples
///
/// ```
/// use std::collections::HashMap;
/// use ironflow_store::entities::{NewRun, TriggerKind};
/// use serde_json::json;
///
/// let req = NewRun {
///     workflow_name: "deploy".to_string(),
///     trigger: TriggerKind::Manual,
///     payload: json!({}),
///     max_retries: 3,
///     handler_version: None,
///     labels: HashMap::new(),
///     scheduled_at: None,
///     idempotency_key: None,
/// };
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewRun {
    /// Workflow name.
    pub workflow_name: String,
    /// How the run was triggered.
    pub trigger: TriggerKind,
    /// Trigger-specific payload.
    pub payload: Value,
    /// Maximum retry attempts.
    pub max_retries: u32,
    /// Version of the handler at the time of run creation.
    pub handler_version: Option<String>,
    /// User-defined key-value labels for categorization and filtering.
    #[serde(default)]
    pub labels: HashMap<String, String>,
    /// When the run should start executing. `None` means immediately.
    #[serde(default)]
    pub scheduled_at: Option<DateTime<Utc>>,
    /// Optional idempotency key binding this request to a single run.
    ///
    /// When set and already bound to a run created within [`IDEMPOTENCY_WINDOW`],
    /// the store returns that run instead of inserting a new one.
    #[serde(default)]
    pub idempotency_key: Option<String>,
}

/// Filters for listing runs.
///
/// All fields are optional; `None` means "no filter" for that field.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::{RunFilter, RunStatus};
///
/// let filter = RunFilter {
///     workflow_name: Some("deploy".to_string()),
///     status: Some(RunStatus::Completed),
///     ..RunFilter::default()
/// };
/// ```
#[derive(Debug, Clone, Default)]
pub struct RunFilter {
    /// Filter by workflow name (exact match).
    pub workflow_name: Option<String>,
    /// Filter by run status.
    pub status: Option<RunStatus>,
    /// Only include runs created after this timestamp.
    pub created_after: Option<DateTime<Utc>>,
    /// Only include runs created before this timestamp.
    pub created_before: Option<DateTime<Utc>>,
    /// When `Some(true)`, only include runs that have at least one step.
    /// When `Some(false)`, only include runs with no steps.
    /// When `None`, no filtering on steps.
    pub has_steps: Option<bool>,
    /// Filter by label key-value pair. Only include runs that have ALL specified labels.
    pub labels: Option<HashMap<String, String>>,
}

/// Partial update for a run.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::{RunUpdate, RunStatus};
///
/// let update = RunUpdate {
///     status: Some(RunStatus::Completed),
///     ..RunUpdate::default()
/// };
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RunUpdate {
    /// New status.
    pub status: Option<RunStatus>,
    /// Error message.
    pub error: Option<String>,
    /// Increment retry count.
    pub increment_retry: bool,
    /// Aggregated cost.
    pub cost_usd: Option<Decimal>,
    /// Aggregated duration.
    pub duration_ms: Option<u64>,
    /// When execution started.
    pub started_at: Option<DateTime<Utc>>,
    /// When execution completed.
    pub completed_at: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use serde_json::json;

    #[test]
    fn newrun_serde_roundtrip() {
        let new_run = NewRun {
            workflow_name: "deploy".to_string(),
            trigger: TriggerKind::Manual,
            payload: json!({"key": "value"}),
            max_retries: 3,
            handler_version: Some("1.2.0".to_string()),
            labels: HashMap::from([("env".to_string(), "prod".to_string())]),
            scheduled_at: None,
            idempotency_key: None,
        };

        let json = serde_json::to_string(&new_run).expect("serialize");
        let back: NewRun = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.workflow_name, new_run.workflow_name);
        assert_eq!(back.trigger, new_run.trigger);
        assert_eq!(back.payload, new_run.payload);
        assert_eq!(back.max_retries, new_run.max_retries);
        assert_eq!(back.handler_version, new_run.handler_version);
        assert_eq!(back.labels, new_run.labels);
        assert_eq!(back.scheduled_at, new_run.scheduled_at);
        assert_eq!(back.idempotency_key, new_run.idempotency_key);
    }

    #[test]
    fn run_serde_preserves_all_fields() {
        use crate::entities::FsmState;
        use chrono::Utc;
        use uuid::Uuid;

        let now = Utc::now();
        let run = Run {
            id: Uuid::now_v7(),
            workflow_name: "test-wf".to_string(),
            status: FsmState::new(RunStatus::Running, Uuid::now_v7()),
            trigger: TriggerKind::Webhook {
                path: "/hooks/test".to_string(),
            },
            payload: json!({"data": 123}),
            error: Some("test error".to_string()),
            retry_count: 2,
            max_retries: 5,
            cost_usd: Decimal::new(1234, 2),
            duration_ms: 5000,
            created_at: now,
            updated_at: now,
            started_at: Some(now),
            completed_at: Some(now),
            handler_version: Some("2.0.0".to_string()),
            labels: HashMap::from([
                ("env".to_string(), "staging".to_string()),
                ("team".to_string(), "platform".to_string()),
            ]),
            scheduled_at: Some(now),
            idempotency_key: Some("gh:abc-123".to_string()),
        };

        let json = serde_json::to_string(&run).expect("serialize");
        let back: Run = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(back.id, run.id);
        assert_eq!(back.workflow_name, run.workflow_name);
        assert_eq!(back.status.state, run.status.state);
        assert_eq!(back.trigger, run.trigger);
        assert_eq!(back.payload, run.payload);
        assert_eq!(back.error, run.error);
        assert_eq!(back.retry_count, run.retry_count);
        assert_eq!(back.max_retries, run.max_retries);
        assert_eq!(back.cost_usd, run.cost_usd);
        assert_eq!(back.duration_ms, run.duration_ms);
        assert_eq!(back.started_at, run.started_at);
        assert_eq!(back.completed_at, run.completed_at);
        assert_eq!(back.handler_version, run.handler_version);
        assert_eq!(back.labels, run.labels);
        assert_eq!(back.scheduled_at, run.scheduled_at);
        assert_eq!(back.idempotency_key, run.idempotency_key);
    }

    #[test]
    fn runupdate_serde_roundtrip() {
        let update = RunUpdate {
            status: Some(RunStatus::Completed),
            error: Some("test error".to_string()),
            increment_retry: true,
            cost_usd: Some(Decimal::new(5000, 2)),
            duration_ms: Some(3000),
            started_at: None,
            completed_at: None,
        };

        let json = serde_json::to_string(&update).expect("serialize");
        let back: RunUpdate = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(back.status, update.status);
        assert_eq!(back.error, update.error);
        assert_eq!(back.increment_retry, update.increment_retry);
        assert_eq!(back.cost_usd, update.cost_usd);
        assert_eq!(back.duration_ms, update.duration_ms);
    }

    #[test]
    fn runfilter_default_is_no_filters() {
        let filter = RunFilter::default();
        assert!(filter.workflow_name.is_none());
        assert!(filter.status.is_none());
        assert!(filter.created_after.is_none());
        assert!(filter.created_before.is_none());
    }

    #[test]
    fn runfilter_with_multiple_criteria() {
        let filter = RunFilter {
            workflow_name: Some("deploy".to_string()),
            status: Some(RunStatus::Running),
            ..RunFilter::default()
        };

        assert_eq!(filter.workflow_name, Some("deploy".to_string()));
        assert_eq!(filter.status, Some(RunStatus::Running));
        assert!(filter.created_after.is_none());
        assert!(filter.created_before.is_none());
    }
}
