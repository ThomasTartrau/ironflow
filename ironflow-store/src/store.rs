//! The [`RunStore`] trait — async storage abstraction for runs and steps.
//!
//! Implement this trait to plug in any backing store. Built-in implementations:
//!
//! - [`InMemoryStore`](crate::memory::InMemoryStore) — development and testing.
//! - `PostgresStore` — production (behind the `store-postgres` feature).

use std::future::Future;
use std::pin::Pin;

use uuid::Uuid;

use crate::api_key_store::ApiKeyStore;
use crate::audit_log_store::AuditLogStore;
use crate::entities::{
    NewRun, NewStep, NewStepDependency, Page, Run, RunCreation, RunFilter, RunStats, RunStatus,
    RunUpdate, Step, StepDependency, StepUpdate,
};
use crate::error::StoreError;
use crate::secret_store::SecretStore;
use crate::user_store::UserStore;

/// Boxed future for [`RunStore`] methods — ensures object safety for `dyn RunStore`.
pub type StoreFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, StoreError>> + Send + 'a>>;

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
    /// Returns `None` if no pending runs are available.
    fn pick_next_pending(&self) -> StoreFuture<'_, Option<Run>>;

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
///     idempotency_key: None,
///     max_cost_usd: None,
/// }).await?.into_run();
/// let _users = store.count_users().await?;
/// # Ok(())
/// # }
/// ```
pub trait Store: RunStore + UserStore + ApiKeyStore + SecretStore + AuditLogStore {}

impl<T: RunStore + UserStore + ApiKeyStore + SecretStore + AuditLogStore> Store for T {}
