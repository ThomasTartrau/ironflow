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
use crate::artifact_store::ArtifactStore;
use crate::audit_log_store::AuditLogStore;
use crate::entities::{
    LeaseRequest, NewRun, NewStep, NewStepDependency, Page, PurgePolicy, PurgeableRun, ReapedRun,
    Run, RunCreation, RunFilter, RunStats, RunStatus, RunUpdate, Step, StepDependency, StepUpdate,
};
use crate::error::StoreError;
use crate::secret_store::SecretStore;
use crate::user_store::UserStore;

/// Boxed future for [`RunStore`] methods — ensures object safety for `dyn RunStore`.
pub type StoreFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, StoreError>> + Send + 'a>>;

/// Error recorded on a run that exhausted its retries through lease expiries.
///
/// Set by [`RunStore::reap_expired_leases`] when a run has been recovered more
/// than `max_retries` times.
pub const LEASE_EXPIRED_ERROR: &str = "worker lease expired";

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
///     max_cost_usd: None,
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
    /// # Errors
    ///
    /// Returns [`StoreError::RunNotFound`] if the run does not exist.
    fn update_run(&self, id: Uuid, update: RunUpdate) -> StoreFuture<'_, ()>;

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
    /// Returns `None` if no pending runs are available.
    fn pick_next_pending(&self, lease: Option<LeaseRequest>) -> StoreFuture<'_, Option<Run>>;

    /// Extend the worker lease on a run and return the new expiry.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::RunNotFound`] if the run does not exist.
    /// Returns [`StoreError::LeaseLost`] if the run is no longer `Running` or if
    /// the lease belongs to another worker — the caller must stop executing it.
    fn renew_lease(&self, id: Uuid, lease: LeaseRequest) -> StoreFuture<'_, DateTime<Utc>>;

    /// Recover runs whose worker lease expired, at most `limit` per call.
    ///
    /// Each recovered run has its retry count incremented and its lease cleared,
    /// then goes back to `Pending` — or to `Failed` with `worker lease expired`
    /// once `max_retries` is exhausted. Runs without a lease are never touched.
    ///
    /// The whole batch is atomic per run (`FOR UPDATE SKIP LOCKED` in
    /// PostgreSQL), so concurrent reapers never recover the same run twice.
    ///
    /// Callers are responsible for the side effects that follow a recovery:
    /// failing orphaned steps and publishing status-change events.
    fn reap_expired_leases(&self, limit: u32) -> StoreFuture<'_, Vec<ReapedRun>>;

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

    /// Get aggregated statistics across runs matching the filter.
    ///
    /// Returns counts of runs by terminal state, counts of active runs,
    /// and totals for cost and duration. Computed efficiently by the store
    /// implementation (single SQL query in PostgreSQL).
    ///
    /// Pass [`RunFilter::default()`] to get stats across all runs.
    fn get_stats(&self, filter: RunFilter) -> StoreFuture<'_, RunStats>;

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
///     max_cost_usd: None,
/// }).await?.into_run();
/// let _users = store.count_users().await?;
/// # Ok(())
/// # }
/// ```
pub trait Store:
    RunStore + UserStore + ApiKeyStore + SecretStore + AuditLogStore + ArtifactStore
{
}

impl<T: RunStore + UserStore + ApiKeyStore + SecretStore + AuditLogStore + ArtifactStore> Store
    for T
{
}
