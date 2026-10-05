//! [`Run`] entity and related request/update types.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

use super::{FsmState, RunActor, RunStatus, TriggerKind};

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
    /// Number of times this run has been retried after a handler failure.
    ///
    /// Each retry starts a new attempt (`retry_count + 1`). A recovery after a
    /// lost worker lease does not change it: see [`Run::lease_recoveries`].
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
    /// The authenticated principal that created this run.
    ///
    /// `None` for cron, webhook, and programmatic triggers.
    #[serde(default)]
    pub created_by: Option<RunActor>,
    /// Human-readable label for [`Run::created_by`].
    ///
    /// Read-only projection resolved at read time from the referenced user and
    /// API key — never written by [`crate::store::RunStore::create_run`]. `None`
    /// when there is no actor, or when the referenced user or key no longer exists.
    #[serde(default)]
    pub created_by_label: Option<String>,
    /// Client-supplied idempotency key that produced this run, if any.
    ///
    /// See [`IDEMPOTENCY_WINDOW`] for how long a key stays bound to its run.
    #[serde(default)]
    pub idempotency_key: Option<String>,
    /// Concurrency key held by this run while it is not terminal, if any.
    ///
    /// See [`NewRun::concurrency_key`] for the exclusivity rule.
    #[serde(default)]
    pub concurrency_key: Option<String>,
    /// Concurrency groups this run belongs to, each with its own limit.
    ///
    /// See [`NewRun::concurrency_limits`] for the gating rule. Empty means the
    /// run is not limited by any group.
    #[serde(default)]
    pub concurrency_limits: Vec<ConcurrencyLimit>,
    /// Maximum cumulative cost allowed for this run, in USD.
    ///
    /// Resolved once at run creation and frozen for the lifetime of the run.
    /// `None` means no cap.
    #[serde(default)]
    pub max_cost_usd: Option<Decimal>,
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
    /// Output the workflow handler set with `WorkflowContext::set_output`.
    ///
    /// Written when an execution ends (completed, warning, failed or
    /// cancelled). `None` when the handler never set an output.
    #[serde(default)]
    pub output: Option<Value>,
    /// Number of times the reaper recovered this run after its worker lease
    /// expired.
    ///
    /// Bounded by [`Run::max_retries`] and counted independently of
    /// [`Run::retry_count`]: a recovered run stays in the same attempt, so the
    /// steps it already finished are replayed instead of executed again.
    #[serde(default)]
    pub lease_recoveries: u32,
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

/// Maximum accepted length of a concurrency key, in bytes.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::MAX_CONCURRENCY_KEY_LEN;
///
/// assert_eq!(MAX_CONCURRENCY_KEY_LEN, 255);
/// ```
pub const MAX_CONCURRENCY_KEY_LEN: usize = 255;

/// Maximum accepted length of a concurrency group name, in bytes.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::MAX_CONCURRENCY_GROUP_LEN;
///
/// assert_eq!(MAX_CONCURRENCY_GROUP_LEN, 255);
/// ```
pub const MAX_CONCURRENCY_GROUP_LEN: usize = 255;

/// Membership of a run in a concurrency group, with the limit the run accepts.
///
/// A run is only moved to `Running` while fewer than `limit` root runs
/// carrying `group` are running. Each run is compared against its own limit,
/// so two runs of the same group may carry different limits.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::ConcurrencyLimit;
///
/// let limit = ConcurrencyLimit::new("repo:acme/api", 2);
/// assert_eq!(limit.group, "repo:acme/api");
/// assert_eq!(limit.limit, 2);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ConcurrencyLimit {
    /// Name of the concurrency group (1 to 255 bytes).
    pub group: String,
    /// Maximum number of root runs of this group running at once (at least 1).
    pub limit: u32,
}

impl ConcurrencyLimit {
    /// Build a concurrency limit. Validation happens in
    /// [`validate_concurrency_limits`].
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_store::entities::ConcurrencyLimit;
    ///
    /// let limit = ConcurrencyLimit::new("tenant:42", 1);
    /// assert_eq!(limit.limit, 1);
    /// ```
    pub fn new(group: impl Into<String>, limit: u32) -> Self {
        Self {
            group: group.into(),
            limit,
        }
    }
}

/// Why a list of concurrency limits was refused.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::ConcurrencyLimitError;
///
/// let err = ConcurrencyLimitError::EmptyGroup;
/// assert_eq!(err.to_string(), "concurrency group must not be empty");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum ConcurrencyLimitError {
    /// A group name was empty or whitespace only.
    #[error("concurrency group must not be empty")]
    EmptyGroup,
    /// A group name exceeds [`MAX_CONCURRENCY_GROUP_LEN`] bytes.
    #[error("concurrency group '{group}' exceeds {max} bytes")]
    GroupTooLong {
        /// The offending group name.
        group: String,
        /// The maximum accepted length, in bytes.
        max: usize,
    },
    /// A limit was zero, which would hold the run back forever.
    #[error("concurrency limit for group '{group}' must be at least 1")]
    ZeroLimit {
        /// The group carrying the zero limit.
        group: String,
    },
    /// The same group appears twice in one run.
    #[error("concurrency group '{group}' is listed more than once")]
    DuplicateGroup {
        /// The duplicated group name.
        group: String,
    },
}

/// Validate the concurrency limits of a run before persisting it.
///
/// # Errors
///
/// Returns [`ConcurrencyLimitError::EmptyGroup`] for an empty or
/// whitespace-only group, [`ConcurrencyLimitError::GroupTooLong`] for a group
/// longer than [`MAX_CONCURRENCY_GROUP_LEN`] bytes,
/// [`ConcurrencyLimitError::ZeroLimit`] for a limit of zero and
/// [`ConcurrencyLimitError::DuplicateGroup`] when a group is listed twice.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::{ConcurrencyLimit, validate_concurrency_limits};
///
/// assert!(validate_concurrency_limits(&[]).is_ok());
/// assert!(validate_concurrency_limits(&[ConcurrencyLimit::new("repo:acme", 2)]).is_ok());
/// assert!(validate_concurrency_limits(&[ConcurrencyLimit::new("repo:acme", 0)]).is_err());
/// ```
pub fn validate_concurrency_limits(
    limits: &[ConcurrencyLimit],
) -> Result<(), ConcurrencyLimitError> {
    let mut seen: HashSet<&str> = HashSet::with_capacity(limits.len());
    for limit in limits {
        if limit.group.trim().is_empty() {
            return Err(ConcurrencyLimitError::EmptyGroup);
        }
        if limit.group.len() > MAX_CONCURRENCY_GROUP_LEN {
            return Err(ConcurrencyLimitError::GroupTooLong {
                group: limit.group.clone(),
                max: MAX_CONCURRENCY_GROUP_LEN,
            });
        }
        if limit.limit == 0 {
            return Err(ConcurrencyLimitError::ZeroLimit {
                group: limit.group.clone(),
            });
        }
        if !seen.insert(limit.group.as_str()) {
            return Err(ConcurrencyLimitError::DuplicateGroup {
                group: limit.group.clone(),
            });
        }
    }
    Ok(())
}

/// Number of due runs held back because a concurrency group is saturated.
///
/// Produced by
/// [`RunStore::count_blocked_runs_by_group`](crate::store::RunStore::count_blocked_runs_by_group).
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::ConcurrencyGroupBacklog;
///
/// let backlog = ConcurrencyGroupBacklog {
///     group: "repo:acme/api".to_string(),
///     blocked_runs: 3,
/// };
/// assert_eq!(backlog.blocked_runs, 3);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ConcurrencyGroupBacklog {
    /// Name of the saturated concurrency group.
    pub group: String,
    /// Number of due pending or retrying runs held back by this group.
    pub blocked_runs: u64,
}

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
///     created_by: None,
///     idempotency_key: Some("deploy-2026-07-26".to_string()),
///     concurrency_key: None,
///     concurrency_limits: Vec::new(),
///     max_cost_usd: None,
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
///     created_by: None,
///     idempotency_key: None,
///     concurrency_key: None,
///     concurrency_limits: Vec::new(),
///     max_cost_usd: None,
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
    /// The authenticated principal creating this run.
    ///
    /// Defaults to `None` when absent from the payload, so an older worker that
    /// does not send the field keeps working against a newer API.
    #[serde(default)]
    pub created_by: Option<RunActor>,
    /// Optional idempotency key binding this request to a single run.
    ///
    /// When set and already bound to a run created within [`IDEMPOTENCY_WINDOW`],
    /// the store returns that run instead of inserting a new one.
    #[serde(default)]
    pub idempotency_key: Option<String>,
    /// Optional concurrency key making this run exclusive.
    ///
    /// At most one non-terminal run may hold a given key; creation fails with
    /// [`StoreError::ConcurrencyConflict`](crate::error::StoreError::ConcurrencyConflict)
    /// otherwise. The key is released when the run reaches a terminal state.
    #[serde(default)]
    pub concurrency_key: Option<String>,
    /// Concurrency groups this run belongs to, each with its own limit.
    ///
    /// The run is only moved to `Running` while, for every listed group, fewer
    /// than its `limit` root runs carrying that group are running. Sub-workflow
    /// runs never count. Empty means no group limit.
    #[serde(default)]
    pub concurrency_limits: Vec<ConcurrencyLimit>,
    /// Maximum cumulative cost allowed for this run, in USD. `None` means no cap.
    #[serde(default)]
    pub max_cost_usd: Option<Decimal>,
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
    /// Filter by author. Matches runs created by this user directly, and runs
    /// created by one of this user's API keys.
    pub created_by_user_id: Option<Uuid>,
    /// Filter by concurrency group. Only include runs whose concurrency limits
    /// contain this group.
    pub concurrency_group: Option<String>,
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
    /// When the run should next be picked up. Used to arm the retry backoff.
    #[serde(default)]
    pub scheduled_at: Option<DateTime<Utc>>,
    /// Output set by the workflow handler. `None` leaves the stored output unchanged.
    #[serde(default)]
    pub output: Option<Value>,
}

/// Retention policy for purging old runs.
///
/// Runs are eligible for purging when they are in a terminal state
/// ([`RunStatus::is_terminal`]) **and** exceed either the age limit or the
/// per-workflow count limit.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::PurgePolicy;
///
/// let policy = PurgePolicy {
///     max_age_days: 90,
///     max_runs_per_workflow: 1000,
///     dry_run: false,
/// };
/// assert_eq!(policy.max_age_days, 90);
/// ```
#[derive(Debug, Clone)]
pub struct PurgePolicy {
    /// Runs older than this many days are eligible for purging.
    pub max_age_days: u32,
    /// When a workflow has more runs than this, the oldest terminal runs are
    /// eligible for purging.
    pub max_runs_per_workflow: u32,
    /// When `true`, the purger logs what would be deleted but does not delete.
    pub dry_run: bool,
}

/// Why a run was selected for purging.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::PurgeReason;
///
/// let reason = PurgeReason::TooOld;
/// assert_eq!(format!("{reason}"), "too_old");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PurgeReason {
    /// The run exceeded [`PurgePolicy::max_age_days`].
    TooOld,
    /// The workflow exceeded [`PurgePolicy::max_runs_per_workflow`].
    ExceedsWorkflowLimit,
}

impl fmt::Display for PurgeReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PurgeReason::TooOld => f.write_str("too_old"),
            PurgeReason::ExceedsWorkflowLimit => f.write_str("exceeds_workflow_limit"),
        }
    }
}

/// A run selected for purging by [`RunStore::list_purgeable_runs`](crate::store::RunStore::list_purgeable_runs).
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::{PurgeReason, PurgeableRun};
/// use uuid::Uuid;
///
/// let purgeable = PurgeableRun {
///     run_id: Uuid::now_v7(),
///     workflow_name: "deploy".to_string(),
///     reason: PurgeReason::TooOld,
/// };
/// assert_eq!(purgeable.reason, PurgeReason::TooOld);
/// ```
#[derive(Debug, Clone)]
pub struct PurgeableRun {
    /// The run to purge.
    pub run_id: Uuid,
    /// Workflow the run belongs to.
    pub workflow_name: String,
    /// Why this run was selected.
    pub reason: PurgeReason,
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use serde_json::json;

    #[test]
    fn newrun_serde_roundtrip() {
        let new_run = NewRun {
            created_by: None,
            workflow_name: "deploy".to_string(),
            trigger: TriggerKind::Manual,
            payload: json!({"key": "value"}),
            max_retries: 3,
            handler_version: Some("1.2.0".to_string()),
            labels: HashMap::from([("env".to_string(), "prod".to_string())]),
            scheduled_at: None,
            idempotency_key: None,
            concurrency_key: None,
            concurrency_limits: Vec::new(),
            max_cost_usd: Some(Decimal::new(250, 2)),
        };

        let json = serde_json::to_string(&new_run).expect("serialize");
        let back: NewRun = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.max_cost_usd, new_run.max_cost_usd);
        assert_eq!(back.workflow_name, new_run.workflow_name);
        assert_eq!(back.trigger, new_run.trigger);
        assert_eq!(back.payload, new_run.payload);
        assert_eq!(back.max_retries, new_run.max_retries);
        assert_eq!(back.handler_version, new_run.handler_version);
        assert_eq!(back.labels, new_run.labels);
        assert_eq!(back.scheduled_at, new_run.scheduled_at);
        assert_eq!(back.created_by, new_run.created_by);
        assert_eq!(back.idempotency_key, new_run.idempotency_key);
    }

    #[test]
    fn newrun_serde_roundtrip_with_actor() {
        let actor = RunActor::ApiKey {
            api_key_id: Uuid::now_v7(),
            user_id: Uuid::now_v7(),
        };
        let new_run = NewRun {
            workflow_name: "deploy".to_string(),
            trigger: TriggerKind::Api,
            payload: json!({}),
            max_retries: 0,
            handler_version: None,
            labels: HashMap::new(),
            scheduled_at: None,
            created_by: Some(actor.clone()),
            idempotency_key: None,
            concurrency_key: None,
            concurrency_limits: Vec::new(),
            max_cost_usd: None,
        };

        let json = serde_json::to_string(&new_run).expect("serialize");
        let back: NewRun = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.created_by, Some(actor));
    }

    #[test]
    fn newrun_deserializes_without_created_by() {
        // An older worker POSTs a payload with no `created_by` field.
        let raw = json!({
            "workflow_name": "deploy",
            "trigger": {"kind": "workflow"},
            "payload": {},
            "max_retries": 0,
            "handler_version": null,
        });

        let new_run: NewRun = serde_json::from_value(raw).expect("deserialize");
        assert!(new_run.created_by.is_none());
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
            created_by: Some(RunActor::User {
                user_id: Uuid::now_v7(),
            }),
            created_by_label: Some("alice".to_string()),
            idempotency_key: Some("gh:abc-123".to_string()),
            concurrency_key: Some("issue:12".to_string()),
            concurrency_limits: vec![ConcurrencyLimit::new("repo:acme", 2)],
            max_cost_usd: Some(Decimal::new(500, 2)),
            worker_id: Some("worker-1".to_string()),
            lease_expires_at: Some(now),
            output: Some(json!({"verdict": "approved", "score": 9})),
            lease_recoveries: 1,
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
        assert_eq!(back.created_by, run.created_by);
        assert_eq!(back.created_by_label, run.created_by_label);
        assert_eq!(back.idempotency_key, run.idempotency_key);
        assert_eq!(back.concurrency_key, run.concurrency_key);
        assert_eq!(back.concurrency_limits, run.concurrency_limits);
        assert_eq!(back.max_cost_usd, run.max_cost_usd);
        assert_eq!(back.worker_id, run.worker_id);
        assert_eq!(back.lease_expires_at, run.lease_expires_at);
        assert_eq!(back.output, run.output);
        assert_eq!(back.lease_recoveries, run.lease_recoveries);
    }

    #[test]
    fn run_without_output_field_deserializes_to_none_output() {
        // A run serialized before the `output` column existed has no such key.
        let now = Utc::now();
        let run = Run {
            id: Uuid::now_v7(),
            workflow_name: "legacy".to_string(),
            status: FsmState::new(RunStatus::Completed, Uuid::now_v7()),
            trigger: TriggerKind::Manual,
            payload: json!({}),
            error: None,
            retry_count: 0,
            max_retries: 0,
            cost_usd: Decimal::ZERO,
            duration_ms: 0,
            created_at: now,
            updated_at: now,
            started_at: None,
            completed_at: None,
            handler_version: None,
            labels: HashMap::new(),
            scheduled_at: None,
            created_by: None,
            created_by_label: None,
            idempotency_key: None,
            concurrency_key: None,
            concurrency_limits: Vec::new(),
            max_cost_usd: None,
            worker_id: None,
            lease_expires_at: None,
            output: Some(json!("set")),
            lease_recoveries: 2,
        };
        let mut raw = serde_json::to_value(&run).expect("serialize");
        raw.as_object_mut().expect("object").remove("output");
        raw.as_object_mut()
            .expect("object")
            .remove("lease_recoveries");

        let back: Run = serde_json::from_value(raw).expect("deserialize");
        assert!(back.output.is_none());
        assert_eq!(back.lease_recoveries, 0);
    }

    #[test]
    fn runupdate_output_round_trips_and_defaults_to_none() {
        let update = RunUpdate {
            output: Some(json!({"verdict": "rejected"})),
            ..RunUpdate::default()
        };
        let json = serde_json::to_string(&update).expect("serialize");
        let back: RunUpdate = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.output, update.output);

        let mut legacy = serde_json::to_value(RunUpdate::default()).expect("serialize");
        legacy.as_object_mut().expect("object").remove("output");
        let parsed: RunUpdate = serde_json::from_value(legacy).expect("deserialize");
        assert!(parsed.output.is_none());
    }

    #[test]
    fn newrun_max_cost_usd_defaults_to_none_when_absent() {
        let without_cap = NewRun {
            workflow_name: "deploy".to_string(),
            trigger: TriggerKind::Manual,
            payload: json!({}),
            max_retries: 0,
            handler_version: None,
            labels: HashMap::new(),
            scheduled_at: None,
            created_by: None,
            idempotency_key: None,
            concurrency_key: None,
            concurrency_limits: Vec::new(),
            max_cost_usd: None,
        };
        let mut value = serde_json::to_value(&without_cap).expect("serialize");
        value
            .as_object_mut()
            .expect("object")
            .remove("max_cost_usd");

        let parsed: NewRun = serde_json::from_value(value).expect("deserialize");
        assert!(parsed.max_cost_usd.is_none());
    }

    #[test]
    fn newrun_concurrency_limits_default_to_empty_when_absent() {
        let raw = json!({
            "workflow_name": "deploy",
            "trigger": {"kind": "manual"},
            "payload": {},
            "max_retries": 0,
            "handler_version": null,
        });

        let new_run: NewRun = serde_json::from_value(raw).expect("deserialize");
        assert!(new_run.concurrency_limits.is_empty());
    }

    #[test]
    fn newrun_serde_roundtrip_keeps_concurrency_limits() {
        let new_run = NewRun {
            workflow_name: "deploy".to_string(),
            trigger: TriggerKind::Manual,
            payload: json!({}),
            max_retries: 0,
            handler_version: None,
            labels: HashMap::new(),
            scheduled_at: None,
            created_by: None,
            idempotency_key: None,
            concurrency_key: None,
            concurrency_limits: vec![
                ConcurrencyLimit::new("repo:acme", 2),
                ConcurrencyLimit::new("tenant:42", 5),
            ],
            max_cost_usd: None,
        };

        let json = serde_json::to_string(&new_run).expect("serialize");
        let back: NewRun = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.concurrency_limits, new_run.concurrency_limits);
    }

    #[test]
    fn validate_concurrency_limits_accepts_empty_and_valid() {
        assert_eq!(validate_concurrency_limits(&[]), Ok(()));
        assert_eq!(
            validate_concurrency_limits(&[
                ConcurrencyLimit::new("repo:acme", 1),
                ConcurrencyLimit::new("tenant:42", 10),
            ]),
            Ok(())
        );
    }

    #[test]
    fn validate_concurrency_limits_rejects_empty_group() {
        assert_eq!(
            validate_concurrency_limits(&[ConcurrencyLimit::new("", 1)]),
            Err(ConcurrencyLimitError::EmptyGroup)
        );
        assert_eq!(
            validate_concurrency_limits(&[ConcurrencyLimit::new("   ", 1)]),
            Err(ConcurrencyLimitError::EmptyGroup)
        );
    }

    #[test]
    fn validate_concurrency_limits_rejects_zero_limit() {
        assert_eq!(
            validate_concurrency_limits(&[ConcurrencyLimit::new("repo:acme", 0)]),
            Err(ConcurrencyLimitError::ZeroLimit {
                group: "repo:acme".to_string(),
            })
        );
    }

    #[test]
    fn validate_concurrency_limits_rejects_duplicate_group() {
        assert_eq!(
            validate_concurrency_limits(&[
                ConcurrencyLimit::new("repo:acme", 1),
                ConcurrencyLimit::new("repo:acme", 3),
            ]),
            Err(ConcurrencyLimitError::DuplicateGroup {
                group: "repo:acme".to_string(),
            })
        );
    }

    #[test]
    fn validate_concurrency_limits_rejects_too_long_group() {
        let at_max = "g".repeat(MAX_CONCURRENCY_GROUP_LEN);
        assert_eq!(
            validate_concurrency_limits(&[ConcurrencyLimit::new(at_max, 1)]),
            Ok(())
        );

        let too_long = "g".repeat(MAX_CONCURRENCY_GROUP_LEN + 1);
        assert_eq!(
            validate_concurrency_limits(&[ConcurrencyLimit::new(too_long.clone(), 1)]),
            Err(ConcurrencyLimitError::GroupTooLong {
                group: too_long,
                max: MAX_CONCURRENCY_GROUP_LEN,
            })
        );
    }

    #[test]
    fn validate_concurrency_limits_accepts_unicode_group() {
        let group = "d\u{e9}p\u{f4}t:caf\u{e9}-\u{2615}";
        assert_eq!(
            validate_concurrency_limits(&[ConcurrencyLimit::new(group, 2)]),
            Ok(())
        );
        // Length is counted in bytes: 128 two-byte characters exceed 255 bytes.
        let too_long = "\u{e9}".repeat(128);
        assert!(matches!(
            validate_concurrency_limits(&[ConcurrencyLimit::new(too_long, 1)]),
            Err(ConcurrencyLimitError::GroupTooLong { .. })
        ));
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
            scheduled_at: Some(Utc::now()),
            output: None,
        };

        let json = serde_json::to_string(&update).expect("serialize");
        let back: RunUpdate = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(back.status, update.status);
        assert_eq!(back.error, update.error);
        assert_eq!(back.increment_retry, update.increment_retry);
        assert_eq!(back.cost_usd, update.cost_usd);
        assert_eq!(back.duration_ms, update.duration_ms);
        assert_eq!(back.scheduled_at, update.scheduled_at);
    }

    #[test]
    fn runfilter_default_is_no_filters() {
        let filter = RunFilter::default();
        assert!(filter.workflow_name.is_none());
        assert!(filter.status.is_none());
        assert!(filter.created_after.is_none());
        assert!(filter.created_before.is_none());
        assert!(filter.created_by_user_id.is_none());
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
