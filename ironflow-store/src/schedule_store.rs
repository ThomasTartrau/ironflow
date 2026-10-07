//! The [`ScheduleStore`] trait -- async storage abstraction for schedules.

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::entities::{
    NewSchedule, Page, Schedule, ScheduleFiring, ScheduleFiringPlan, ScheduleUpdate,
};
use crate::store::StoreFuture;

/// Async storage abstraction for workflow schedules.
///
/// All methods return a [`StoreFuture`] (boxed future) for object safety,
/// allowing the store to be used as `Arc<dyn ScheduleStore>`.
pub trait ScheduleStore: Send + Sync {
    /// Create a new schedule.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`](crate::error::StoreError) on storage failure.
    fn create_schedule(&self, req: NewSchedule) -> StoreFuture<'_, Schedule>;

    /// Find a schedule by ID. Returns `None` if not found.
    fn find_schedule_by_id(&self, id: Uuid) -> StoreFuture<'_, Option<Schedule>>;

    /// List schedules with pagination, ordered by `created_at` descending.
    fn list_schedules(&self, page: u32, per_page: u32) -> StoreFuture<'_, Page<Schedule>>;

    /// Update a schedule by ID.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::ScheduleNotFound`](crate::error::StoreError) if
    /// the schedule does not exist.
    fn update_schedule(&self, id: Uuid, update: ScheduleUpdate) -> StoreFuture<'_, Schedule>;

    /// Delete a schedule by ID.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::ScheduleNotFound`](crate::error::StoreError) if
    /// the schedule does not exist.
    fn delete_schedule(&self, id: Uuid) -> StoreFuture<'_, ()>;

    /// List active schedules whose `next_trigger_at` is in the past, oldest
    /// first. Changes nothing: fire each one with
    /// [`fire_due_schedule`](Self::fire_due_schedule).
    fn list_due_schedules(&self) -> StoreFuture<'_, Vec<Schedule>>;

    /// Fire a due schedule, atomically.
    ///
    /// In a single transaction: lock the schedule if it is still active and
    /// its `next_trigger_at` still equals `due`, then, for each occurrence of
    /// `plan.occurrences` in order, create its run, built by
    /// [`Schedule::new_run`](crate::entities::Schedule::new_run) with the
    /// idempotency key [`Schedule::occurrence_key`](crate::entities::Schedule::occurrence_key).
    /// An occurrence refused with
    /// [`StoreError::ConcurrencyConflict`](crate::error::StoreError::ConcurrencyConflict)
    /// (a run of the schedule is still active under
    /// [`OverlapPolicy::Skip`](crate::entities::OverlapPolicy::Skip)) writes
    /// nothing and is reported in [`ScheduleFiring::overlapped`]. Then apply
    /// `plan.next`, and set `last_triggered_at` when at least one run was
    /// created or replayed.
    ///
    /// Either all of it is written, or nothing is and the schedule stays due.
    ///
    /// Returns `None` when the schedule is no longer due at `due`: another
    /// instance fired it (or holds its lock), or the schedule was paused,
    /// rescheduled or deleted since it was listed.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`](crate::error::StoreError) when a run or the
    /// schedule cannot be written, for any reason other than a concurrency
    /// conflict. Nothing is written in that case.
    fn fire_due_schedule(
        &self,
        id: Uuid,
        due: DateTime<Utc>,
        plan: ScheduleFiringPlan,
    ) -> StoreFuture<'_, Option<ScheduleFiring>>;
}
