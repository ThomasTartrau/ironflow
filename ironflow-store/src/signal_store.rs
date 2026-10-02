//! The [`SignalStore`] trait -- async storage for signals and the steps waiting
//! for them.

use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

use crate::entities::{
    NewSignal, Page, Signal, SignalFilter, SignalInsert, SignalStepResolution, Step,
};
use crate::store::StoreFuture;

/// Async storage abstraction for signals.
///
/// A signal is an external message, named and keyed, that resumes the runs
/// waiting for it. Waiting runs are represented by a `signal` step in the
/// `Running` state; resolving that step and waking its run happen in one
/// transaction, so a delivery racing a timeout or another delivery resolves
/// the step exactly once.
///
/// All methods return a [`StoreFuture`] (boxed future) for object safety.
///
/// # Examples
///
/// ```no_run
/// use ironflow_store::prelude::*;
/// use serde_json::json;
///
/// # async fn example() -> Result<(), ironflow_store::error::StoreError> {
/// let store = InMemoryStore::new();
///
/// let insert = store.insert_signal(NewSignal {
///     name: "ci.pipeline_finished".to_string(),
///     key: "4f2a9c1".to_string(),
///     payload: json!({"status": "success"}),
///     idempotency_id: Some("delivery-42".to_string()),
/// }).await?;
/// assert!(!insert.is_duplicate());
///
/// let waiters = store.list_signal_waiters("ci.pipeline_finished", "4f2a9c1").await?;
/// assert!(waiters.is_empty());
/// # Ok(())
/// # }
/// ```
pub trait SignalStore: Send + Sync {
    /// Store a signal.
    ///
    /// Idempotent on [`NewSignal::idempotency_id`]: when a signal with the same
    /// ID already exists, nothing is stored and the existing signal is returned
    /// as [`SignalInsert::Duplicate`].
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`](crate::error::StoreError) on storage failure.
    fn insert_signal(&self, signal: NewSignal) -> StoreFuture<'_, SignalInsert>;

    /// List one page of the signals matching `filter`, newest first.
    ///
    /// `page` is 1-based.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`](crate::error::StoreError) on storage failure.
    fn list_signals(
        &self,
        filter: SignalFilter,
        page: u32,
        per_page: u32,
    ) -> StoreFuture<'_, Page<Signal>>;

    /// List the signals named `name` for `key` received at or after `since`,
    /// oldest first, at most 100.
    ///
    /// Used by a run opening a wait step, to find a signal delivered before
    /// the step existed.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`](crate::error::StoreError) on storage failure.
    fn list_signals_for_key(
        &self,
        name: &str,
        key: &str,
        since: DateTime<Utc>,
    ) -> StoreFuture<'_, Vec<Signal>>;

    /// List the `signal` steps currently waiting for `(name, key)`.
    ///
    /// A waiting step is `Running`, has `input.name == name` and
    /// `input.key == key`, and belongs to a run that is `Sleeping`, `Running`
    /// or `Pending`: steps of cancelled or finished runs are excluded.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`](crate::error::StoreError) on storage failure.
    fn list_signal_waiters(&self, name: &str, key: &str) -> StoreFuture<'_, Vec<Step>>;

    /// Resolve a waiting `signal` step with `output`, in one transaction.
    ///
    /// When the step is a `Running` `signal` step, it is completed with
    /// `output`; its run, if `Sleeping`, goes back to `Pending`
    /// (`signal_received`) with `scheduled_at` cleared, and the result is
    /// [`SignalStepResolution::Resolved`]. Otherwise nothing changes and the
    /// step's current output is returned as
    /// [`SignalStepResolution::NotWaiting`].
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::StepNotFound`](crate::error::StoreError::StepNotFound)
    /// if the step does not exist, or a storage error.
    fn resolve_signal_step(
        &self,
        step_id: Uuid,
        output: Value,
    ) -> StoreFuture<'_, SignalStepResolution>;

    /// Suspend a `Running` run on its waiting `signal` step, in one transaction.
    ///
    /// The run goes `Running -> Sleeping` (`delay_started`) and loses its
    /// worker lease. When the step is still waiting, `scheduled_at` is set to
    /// `deadline_at` and `true` is returned. When a signal resolved the step
    /// in the meantime, `scheduled_at` is set to now, so the next waker tick
    /// resumes the run right away, and `false` is returned.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::RunNotFound`](crate::error::StoreError::RunNotFound),
    /// [`StoreError::StepNotFound`](crate::error::StoreError::StepNotFound),
    /// [`StoreError::InvalidTransition`](crate::error::StoreError::InvalidTransition)
    /// if the run is not `Running`, or a storage error.
    fn suspend_run_on_signal(
        &self,
        run_id: Uuid,
        step_id: Uuid,
        deadline_at: DateTime<Utc>,
    ) -> StoreFuture<'_, bool>;

    /// Delete the signals received before `before`. Returns how many were
    /// deleted.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`](crate::error::StoreError) on storage failure.
    fn purge_signals(&self, before: DateTime<Utc>) -> StoreFuture<'_, u64>;
}
