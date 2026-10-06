//! The [`ScheduleStore`] trait -- async storage abstraction for schedules.

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::entities::{NewSchedule, Page, Schedule, ScheduleFiring, ScheduleNext, ScheduleUpdate};
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

    /// Fire one due occurrence of a schedule, atomically.
    ///
    /// In a single transaction: lock the schedule if it is still active and
    /// its `next_trigger_at` still equals `occurrence`, create its run (built
    /// by [`Schedule::new_run`](crate::entities::Schedule::new_run), with the
    /// idempotency key [`Schedule::occurrence_key`](crate::entities::Schedule::occurrence_key)),
    /// set `last_triggered_at` and apply `next`. Either all of it is written,
    /// or nothing is and the schedule stays due.
    ///
    /// Returns `None` when the occurrence is no longer due: another instance
    /// fired it (or holds its lock), or the schedule was paused, rescheduled
    /// or deleted since it was listed.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`](crate::error::StoreError) when the run or the
    /// schedule cannot be written. Nothing is written in that case.
    fn fire_due_schedule(
        &self,
        id: Uuid,
        occurrence: DateTime<Utc>,
        next: ScheduleNext,
    ) -> StoreFuture<'_, Option<ScheduleFiring>>;
}
