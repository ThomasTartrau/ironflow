//! [`Run`] entity and related request/update types.

use std::collections::HashMap;
use std::time::Duration;

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
    /// Identifier of the worker currently holding the lease on this run.
    ///
    /// Set when a worker picks the run up, cleared as soon as the run leaves
    /// `Running`. `None` means no worker owns this run (runs executed inline or
    /// resumed in-process by the API server never hold a lease).
    #[serde(default)]
    pub worker_id: Option<String>,
    /// When the worker lease expires.
    ///
    /// The worker refreshes this while it executes the run. Once it is in the
    /// past, the reaper may requeue the run.
    #[serde(default)]
    pub lease_expires_at: Option<DateTime<Utc>>,
}

/// Request to acquire or renew a worker lease on a run.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
/// use ironflow_store::entities::LeaseRequest;
///
/// let lease = LeaseRequest {
///     worker_id: "worker-1".to_string(),
///     ttl: Duration::from_secs(90),
/// };
/// assert_eq!(lease.ttl.as_secs(), 90);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaseRequest {
    /// Identifier of the worker acquiring the lease.
    pub worker_id: String,
    /// How long the lease stays valid without a refresh.
    pub ttl: Duration,
}

impl LeaseRequest {
    /// Compute the lease expiry from a reference instant.
    ///
    /// A TTL too large to be represented saturates to
    /// [`DateTime::<Utc>::MAX_UTC`] instead of panicking.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    /// use chrono::{TimeZone, Utc};
    /// use ironflow_store::entities::LeaseRequest;
    ///
    /// let lease = LeaseRequest {
    ///     worker_id: "worker-1".to_string(),
    ///     ttl: Duration::from_secs(90),
    /// };
    /// let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    /// assert_eq!(lease.expires_at(now).timestamp(), now.timestamp() + 90);
    /// ```
    pub fn expires_at(&self, from: DateTime<Utc>) -> DateTime<Utc> {
        TimeDelta::from_std(self.ttl)
            .ok()
            .and_then(|ttl| from.checked_add_signed(ttl))
            .unwrap_or(DateTime::<Utc>::MAX_UTC)
    }
}

/// A run recovered by the reaper after its worker lease expired.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::{ReapedRun, RunStatus};
///
/// // Reaped runs are produced by RunStore::reap_expired_leases.
/// fn was_requeued(reaped: &ReapedRun) -> bool {
///     reaped.to == RunStatus::Pending
/// }
/// ```
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ReapedRun {
    /// The run after recovery.
    pub run: Run,
    /// Status the run held before recovery (always [`RunStatus::Running`]).
    pub from: RunStatus,
    /// Status the run was moved to: [`RunStatus::Pending`] when retries remain,
    /// [`RunStatus::Failed`] once `max_retries` is exhausted.
    pub to: RunStatus,
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
            worker_id: Some("worker-1".to_string()),
            lease_expires_at: Some(now),
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
        assert_eq!(back.worker_id, run.worker_id);
        assert_eq!(back.lease_expires_at, run.lease_expires_at);
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
