//! The [`RunStore`] trait — async storage abstraction for runs and steps.
//!
//! Implement this trait to plug in any backing store. Built-in implementations:
//!
//! - [`InMemoryStore`](crate::memory::InMemoryStore) — development and testing.
//! - `PostgresStore` — production (behind the `store-postgres` feature).

use std::future::Future;
use std::pin::Pin;

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::api_key_store::ApiKeyStore;
use crate::approval_delegation_store::ApprovalDelegationStore;
use crate::artifact_store::ArtifactStore;
use crate::audit_log_store::AuditLogStore;
use crate::entities::{
    ConcurrencyGroupBacklog, LeaseRequest, NewRun, NewStep, NewStepDependency, Page, PurgePolicy,
    PurgeableRun, ReapedRun, Run, RunCreation, RunFilter, RunStats, RunStatus, RunUpdate,
    StatsHistoryBucket, StatsHistoryFilter, Step, StepApproval, StepDependency, StepUpdate,
    WorkerCapabilities,
};
use crate::error::StoreError;
use crate::log_store::LogStore;
use crate::provider_account_store::ProviderAccountStore;
use crate::schedule_store::ScheduleStore;
use crate::secret_store::SecretStore;
use crate::signal_store::SignalStore;
use crate::user_store::UserStore;

/// Boxed future for [`RunStore`] methods — ensures object safety for `dyn RunStore`.
pub type StoreFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, StoreError>> + Send + 'a>>;

/// Error recorded on a run that exhausted its retries through lease expiries.
///
/// Set by [`RunStore::reap_expired_leases`] when a run has been recovered more
/// than `max_retries` times.
pub const LEASE_EXPIRED_ERROR: &str = "worker lease expired";

/// Error recorded on a step that was running when its worker lost the lease.
///
/// Set by the reaper on the `Running` steps of a run that
/// [`RunStore::reap_expired_leases`] requeued. The engine executes such a step
/// again at the same position when the run is picked up, keeping the
/// interrupted record in the step history.
///
/// # Examples
///
/// ```
/// use ironflow_store::store::{LEASE_EXPIRED_ERROR, STEP_INTERRUPTED_ERROR};
///
/// assert_eq!(STEP_INTERRUPTED_ERROR, "interrupted: worker lease lost");
/// assert_ne!(STEP_INTERRUPTED_ERROR, LEASE_EXPIRED_ERROR);
/// ```
pub const STEP_INTERRUPTED_ERROR: &str = "interrupted: worker lease lost";

/// Async storage abstraction for workflow runs and steps.
///
/// All methods return a [`StoreFuture`] (boxed future) to maintain object safety,
/// allowing the store to be used as `Arc<dyn RunStore>`.
///
/// # Examples
///
/// ```no_run
/// use std::collections::HashMap;
/// use ironflow_store::prelude::*;
/// use serde_json::json;
/// use uuid::Uuid;
///
/// # async fn example() -> Result<(), ironflow_store::error::StoreError> {
/// let store = InMemoryStore::new();
///
/// let run = store.create_run(NewRun {
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
///     priority: 0,
///     concurrency_limits: Vec::new(),
///     max_cost_usd: None,
///     worker_tags: Vec::new(),
/// }).await?.into_run();
///
/// let fetched = store.get_run(run.id).await?;
/// assert!(fetched.is_some());
/// # Ok(())
/// # }
/// ```
pub trait RunStore: Send + Sync {
    /// Create a new run in `Pending` status.
    ///
    /// When [`NewRun::idempotency_key`] is set and already bound to a run created
    /// within [`IDEMPOTENCY_WINDOW`](crate::entities::IDEMPOTENCY_WINDOW), nothing is
    /// inserted and that run is returned as [`RunCreation::Existing`]. A key bound to
    /// an older run is released and reused for the new one.
    ///
    /// Concurrent calls sharing the same key resolve to a single run: exactly one
    /// receives [`RunCreation::Created`], the others [`RunCreation::Existing`].
    ///
    /// When [`NewRun::concurrency_key`] is set, the idempotency lookup runs first,
    /// then the key is checked: concurrent calls sharing it are serialized, and
    /// at most one non-terminal run holds it at a time.
    ///
    /// [`NewRun::concurrency_limits`] is validated before anything is written.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::ConcurrencyConflict`](crate::error::StoreError::ConcurrencyConflict)
    /// when a run that is not Completed, Failed, Warning or Cancelled already
    /// holds [`NewRun::concurrency_key`],
    /// [`StoreError::InvalidConcurrencyLimit`](crate::error::StoreError::InvalidConcurrencyLimit)
    /// when [`NewRun::concurrency_limits`] holds an empty or too long group, a
    /// zero limit or a duplicated group, and a database error when the backing
    /// store fails.
    fn create_run(&self, req: NewRun) -> StoreFuture<'_, RunCreation>;

    /// Look up the run bound to an idempotency key.
    ///
    /// Returns `None` when the key is unknown, or when the run holding it is older
    /// than [`IDEMPOTENCY_WINDOW`](crate::entities::IDEMPOTENCY_WINDOW).
    fn find_run_by_idempotency_key(&self, key: &str) -> StoreFuture<'_, Option<Run>>;

    /// Get a run by ID. Returns `None` if not found.
    fn get_run(&self, id: Uuid) -> StoreFuture<'_, Option<Run>>;

    /// List runs matching the given filter, with pagination.
    ///
    /// Results are ordered by `created_at` descending (newest first).
    fn list_runs(&self, filter: RunFilter, page: u32, per_page: u32) -> StoreFuture<'_, Page<Run>>;

    /// Update a run's status with FSM validation.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::InvalidTransition`] if the transition is not allowed.
    /// Returns [`StoreError::RunNotFound`] if the run does not exist.
    fn update_run_status(&self, id: Uuid, new_status: RunStatus) -> StoreFuture<'_, ()>;

    /// Apply a partial update to a run.
    ///
    /// [`RunUpdate::lease`] is applied in the same transaction as the status
    /// transition, after it: `status: Running` with
    /// [`LeaseUpdate::Set`](crate::entities::LeaseUpdate::Set) leaves the run
    /// `Running` and owned by that worker, so it is never `Running` without a
    /// lease in between. [`LeaseUpdate::Release`](crate::entities::LeaseUpdate::Release)
    /// drops the lease without touching the status. An explicit lease change
    /// wins over the clearing that a transition out of `Running` does.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::RunNotFound`] if the run does not exist.
    fn update_run(&self, id: Uuid, update: RunUpdate) -> StoreFuture<'_, ()>;

    /// List the non-terminal descendants of a run, oldest first.
    ///
    /// A descendant is a sub-workflow run
    /// ([`TriggerKind::Workflow`](crate::entities::TriggerKind::Workflow))
    /// reached from `run_id` through
    /// [`PARENT_RUN_ID_LABEL`](crate::entities::PARENT_RUN_ID_LABEL), at any
    /// depth. Terminal runs are not returned, but their own descendants are:
    /// a child left running under a finished parent is still found. Labels are
    /// data, so a chain that loops back on itself is followed once and never
    /// returns `run_id` itself.
    ///
    /// Returns an empty list for an unknown run or a run without children.
    ///
    /// # Errors
    ///
    /// Returns a database error when the backing store fails.
    fn list_active_descendants(&self, run_id: Uuid) -> StoreFuture<'_, Vec<Run>>;

    /// Atomically pick the oldest pending run and transition it to `Running`.
    ///
    /// In PostgreSQL, this uses `SELECT FOR UPDATE SKIP LOCKED` for safe
    /// multi-worker concurrency. The in-memory implementation uses a write lock.
    ///
    /// When `lease` is `Some`, the worker lease is attached in the same
    /// transaction as the status change, so a run is never `Running` without an
    /// owner. Pass `None` for callers that execute runs in-process and cannot
    /// refresh a lease (inline execution, API-side resume): those runs are never
    /// recovered by [`reap_expired_leases`](Self::reap_expired_leases).
    ///
    /// Concurrency groups gate the pick: a run carrying
    /// [`Run::concurrency_limits`] is skipped while, for any of its groups, the
    /// number of root runs in state `Running` carrying that group is already at
    /// or above the run's own limit for it. Sleeping, awaiting approval,
    /// retrying and pending runs do not count, and sub-workflow runs
    /// ([`TriggerKind::Workflow`](crate::entities::TriggerKind::Workflow)) are
    /// never counted. A held-back run does not block the queue: the oldest
    /// eligible run wins. The check is atomic across concurrent callers, so a
    /// group never exceeds its limit.
    ///
    /// Returns `None` if no pending runs are available.
    ///
    /// Equivalent to [`pick_next_pending_for`](Self::pick_next_pending_for)
    /// with no worker capabilities: every run is eligible.
    fn pick_next_pending(&self, lease: Option<LeaseRequest>) -> StoreFuture<'_, Option<Run>> {
        self.pick_next_pending_for(lease, None)
    }

    /// Atomically pick the oldest pending run the worker can take and
    /// transition it to `Running`.
    ///
    /// Same contract as [`pick_next_pending`](Self::pick_next_pending), with
    /// worker routing on top: when `capabilities` is `Some`, a run is only
    /// eligible when [`WorkerCapabilities::can_take`] accepts its workflow name
    /// and its [`Run::worker_tags`]. An ineligible run is skipped and never
    /// blocks younger runs. `None` keeps the legacy behavior of a worker that
    /// sends no capabilities: every run is eligible.
    ///
    /// Returns `None` if no eligible pending run is available.
    fn pick_next_pending_for(
        &self,
        lease: Option<LeaseRequest>,
        capabilities: Option<WorkerCapabilities>,
    ) -> StoreFuture<'_, Option<Run>>;

    /// Extend the worker lease on a run and return the new expiry.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::RunNotFound`] if the run does not exist.
    /// Returns [`StoreError::LeaseLost`] if the run is no longer `Running` or if
    /// the lease belongs to another worker — the caller must stop executing it.
    fn renew_lease(&self, id: Uuid, lease: LeaseRequest) -> StoreFuture<'_, DateTime<Utc>>;

    /// Count, for each concurrency group, the due runs it currently holds back.
    ///
    /// A run is counted when it is pending or retrying, due (no
    /// `scheduled_at` in the future) and not pickable because the group is
    /// saturated for its own limit (see [`pick_next_pending`](Self::pick_next_pending)).
    /// A run held back by two groups counts in both. Groups holding back no
    /// run are omitted. Results are sorted by group name.
    ///
    /// # Errors
    ///
    /// Returns a database error when the backing store fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_store::store::RunStore;
    ///
    /// # async fn example(store: &dyn RunStore) -> Result<(), ironflow_store::error::StoreError> {
    /// for backlog in store.count_blocked_runs_by_group().await? {
    ///     println!("{}: {} runs held back", backlog.group, backlog.blocked_runs);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    fn count_blocked_runs_by_group(&self) -> StoreFuture<'_, Vec<ConcurrencyGroupBacklog>>;

    /// Recover runs whose worker lease expired, at most `limit` per call.
    ///
    /// Each recovered run has [`Run::lease_recoveries`] incremented and its lease
    /// cleared, then goes back to `Pending` — or to `Failed` with
    /// [`LEASE_EXPIRED_ERROR`] once more than `max_retries` recoveries happened.
    /// Runs without a lease are never touched.
    /// A root run resumed through its sub-workflow child carries the lease the
    /// child held (see [`RunUpdate::lease`]), so it is recovered like any run.
    ///
    /// [`Run::retry_count`], and so the attempt number of the steps created
    /// afterwards, is left unchanged: a requeued run resumes in the same attempt
    /// and replays the steps it already finished.
    ///
    /// The whole batch is atomic per run (`FOR UPDATE SKIP LOCKED` in
    /// PostgreSQL), so concurrent reapers never recover the same run twice.
    ///
    /// Callers are responsible for the side effects that follow a recovery:
    /// failing orphaned steps and publishing status-change events.
    fn reap_expired_leases(&self, limit: u32) -> StoreFuture<'_, Vec<ReapedRun>>;

    /// Atomically claim approval steps whose SLA deadline has passed.
    ///
    /// Returns the claimed steps with their *pre-claim* `approval_deadline_at`
    /// still populated, so the caller can report which deadline fired. The
    /// timer is cleared in the same transaction, so a deadline fires at most
    /// once even with several API instances running the escalator (the
    /// PostgreSQL implementation uses `FOR UPDATE SKIP LOCKED`).
    ///
    /// Only steps still in [`StepStatus::AwaitingApproval`](crate::entities::StepStatus::AwaitingApproval)
    /// are returned.
    ///
    /// Delivery is at most once: a caller that crashes between the claim and
    /// the escalation leaves the gate open with no timer, the same trade-off
    /// [`reap_expired_leases`](Self::reap_expired_leases) accepts.
    fn claim_due_approval_deadlines(&self, limit: u32) -> StoreFuture<'_, Vec<Step>>;

    /// Atomically wake the `Sleeping` runs whose `scheduled_at` has passed, at
    /// most `limit` per call.
    ///
    /// Each claimed run goes `Sleeping -> Pending` (`delay_elapsed`) and has
    /// its `scheduled_at` cleared in the same transaction, so a run is woken
    /// exactly once even with several API instances running the waker (the
    /// PostgreSQL implementation uses `FOR UPDATE SKIP LOCKED`). Runs are
    /// claimed oldest `scheduled_at` first.
    ///
    /// Returns the runs as they are after the transition. Callers decide how
    /// the requeued runs resume: a worker picks them up, or the API resumes
    /// them in-process when it has no worker.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] on storage failure.
    fn claim_due_sleeping_runs(&self, limit: u32) -> StoreFuture<'_, Vec<Run>>;

    /// Create a new step for a run.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::RunNotFound`] if the parent run does not exist.
    fn create_step(&self, step: NewStep) -> StoreFuture<'_, Step>;

    /// Apply a partial update to a step after execution.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::StepNotFound`] if the step does not exist.
    fn update_step(&self, id: Uuid, update: StepUpdate) -> StoreFuture<'_, ()>;

    /// Get a single step by ID. Returns `None` if not found.
    fn get_step(&self, id: Uuid) -> StoreFuture<'_, Option<Step>>;

    /// List all steps for a run, ordered by position ascending.
    fn list_steps(&self, run_id: Uuid) -> StoreFuture<'_, Vec<Step>>;

    /// Record a vote on an approval gate and return the updated step.
    ///
    /// The vote is appended atomically to [`Step::approvals`] unless the same
    /// [`StepApproval::user_id`] already voted, in which case the step is
    /// returned unchanged. Recording a vote never resolves the gate: the
    /// caller compares the vote count against the step's
    /// [`approval_requirement`](Step::approval_requirement).
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::StepNotFound`] if the step does not exist.
    fn record_step_approval(&self, step_id: Uuid, approval: StepApproval) -> StoreFuture<'_, Step>;

    /// Get aggregated statistics across runs matching the filter.
    ///
    /// Returns counts of runs by terminal state, counts of active runs
    /// (`Pending`, `Running`, `Retrying`, `AwaitingApproval` or `Sleeping`),
    /// the number of runs awaiting approval, and totals for cost and duration.
    /// Computed efficiently by the store implementation (single SQL query in
    /// PostgreSQL).
    ///
    /// Pass [`RunFilter::default()`] to get stats across all runs.
    fn get_stats(&self, filter: RunFilter) -> StoreFuture<'_, RunStats>;

    /// Get time-bucketed historical statistics for trend charts.
    ///
    /// Aggregates runs created during the filter's period into time buckets
    /// based on its granularity, counting every run status and computing
    /// duration percentiles. Applies the same run filters as
    /// [`get_stats`](Self::get_stats) (workflow substring, status, labels,
    /// steps, author). Bucket boundaries are UTC and weeks start on Monday
    /// (see [`HistoryGranularity::bucket_start`](crate::entities::HistoryGranularity::bucket_start)).
    /// Returns buckets ordered by time ascending; empty buckets are omitted.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] on underlying store failures.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_store::entities::{StatsHistoryFilter, HistoryPeriod, HistoryGranularity};
    /// use ironflow_store::store::RunStore;
    ///
    /// # async fn example(store: &dyn RunStore) -> Result<(), ironflow_store::error::StoreError> {
    /// let filter = StatsHistoryFilter {
    ///     period: HistoryPeriod::SevenDays,
    ///     granularity: HistoryGranularity::OneDay,
    ///     ..StatsHistoryFilter::default()
    /// };
    /// let buckets = store.get_stats_history(filter).await?;
    /// for b in &buckets {
    ///     println!("{}: {} completed, {} failed", b.time, b.completed, b.failed);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    fn get_stats_history(
        &self,
        filter: StatsHistoryFilter,
    ) -> StoreFuture<'_, Vec<StatsHistoryBucket>>;

    /// Create step dependency edges in batch.
    ///
    /// Each entry records that `step_id` depends on `depends_on`.
    /// Duplicate edges are silently ignored.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] if a referenced step does not exist.
    fn create_step_dependencies(&self, deps: Vec<NewStepDependency>) -> StoreFuture<'_, ()>;

    /// List all step dependencies for a given run.
    ///
    /// Returns every edge where either `step_id` or `depends_on` belongs
    /// to the run. Ordered by `created_at` ascending.
    fn list_step_dependencies(&self, run_id: Uuid) -> StoreFuture<'_, Vec<StepDependency>>;

    /// List runs eligible for purging according to the given policy.
    ///
    /// A run is eligible when it is in a terminal state ([`RunStatus::is_terminal`])
    /// **and** either older than `policy.max_age_days` or exceeding
    /// `policy.max_runs_per_workflow` for its workflow (oldest first).
    ///
    /// Runs in non-terminal states (`Pending`, `Running`, `Retrying`,
    /// `AwaitingApproval`) are never returned.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_store::entities::PurgePolicy;
    /// use ironflow_store::store::RunStore;
    ///
    /// # async fn example(store: &dyn RunStore) -> Result<(), ironflow_store::error::StoreError> {
    /// let policy = PurgePolicy { max_age_days: 90, max_runs_per_workflow: 1000, dry_run: false };
    /// let purgeable = store.list_purgeable_runs(&policy, 100).await?;
    /// for p in &purgeable {
    ///     println!("purge {} ({}): {}", p.run_id, p.workflow_name, p.reason);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    fn list_purgeable_runs(
        &self,
        policy: &PurgePolicy,
        batch_size: u32,
    ) -> StoreFuture<'_, Vec<PurgeableRun>>;

    /// Delete a run and all its associated data (steps, step dependencies).
    ///
    /// Returns the `storage_key` of every artifact that belonged to the run,
    /// so the caller can delete the corresponding blobs from the blob store.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::RunNotFound`] if the run does not exist.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_store::store::RunStore;
    /// use uuid::Uuid;
    ///
    /// # async fn example(store: &dyn RunStore, run_id: Uuid) -> Result<(), ironflow_store::error::StoreError> {
    /// let storage_keys = store.delete_run(run_id).await?;
    /// // Caller deletes blobs from the blob store using these keys.
    /// # Ok(())
    /// # }
    /// ```
    fn delete_run(&self, id: Uuid) -> StoreFuture<'_, Vec<String>>;

    /// Apply a partial update to a run and return the updated run.
    ///
    /// Combines [`update_run`](Self::update_run) and [`get_run`](Self::get_run) in
    /// a single operation to avoid an extra round-trip. Store implementations
    /// may override this for efficiency (e.g. reading within the same transaction).
    ///
    /// The default implementation calls `update_run` followed by `get_run`.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::RunNotFound`] if the run does not exist.
    /// Returns [`StoreError::InvalidTransition`] if the status transition is not allowed.
    fn update_run_returning(&self, id: Uuid, update: RunUpdate) -> StoreFuture<'_, Run> {
        Box::pin(async move {
            self.update_run(id, update).await?;
            self.get_run(id).await?.ok_or(StoreError::RunNotFound(id))
        })
    }
}

/// Unified storage abstraction combining all store capabilities.
///
/// Implementors provide runs, steps, users, API keys, and secrets
/// through a single type. Pick one backend (in-memory or PostgreSQL)
/// and it handles everything.
///
/// Both [`InMemoryStore`](crate::memory::InMemoryStore) and
/// [`PostgresStore`](crate::postgres::PostgresStore) implement this trait.
///
/// # Examples
///
/// ```no_run
/// use std::collections::HashMap;
/// use std::sync::Arc;
/// use ironflow_store::prelude::*;
///
/// # async fn example() -> Result<(), ironflow_store::error::StoreError> {
/// let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
///
/// // All capabilities through one reference
/// let _run = store.create_run(NewRun {
///     workflow_name: "deploy".to_string(),
///     trigger: TriggerKind::Manual,
///     payload: serde_json::json!({}),
///     max_retries: 3,
///     handler_version: None,
///     labels: HashMap::new(),
///     scheduled_at: None,
///     created_by: None,
///     idempotency_key: None,
///     concurrency_key: None,
///     priority: 0,
///     concurrency_limits: Vec::new(),
///     max_cost_usd: None,
///     worker_tags: Vec::new(),
/// }).await?.into_run();
/// let _users = store.count_users().await?;
/// # Ok(())
/// # }
/// ```
pub trait Store:
    RunStore
    + UserStore
    + ApiKeyStore
    + SecretStore
    + AuditLogStore
    + ArtifactStore
    + LogStore
    + ScheduleStore
    + ApprovalDelegationStore
    + ProviderAccountStore
    + SignalStore
{
}

impl<
    T: RunStore
        + UserStore
        + ApiKeyStore
        + SecretStore
        + AuditLogStore
        + ArtifactStore
        + LogStore
        + ScheduleStore
        + ApprovalDelegationStore
        + ProviderAccountStore
        + SignalStore,
> Store for T
{
}
