//! The [`ScheduleStore`] trait -- async storage abstraction for schedules.

use uuid::Uuid;

use crate::entities::{NewSchedule, Page, Schedule, ScheduleUpdate};
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

    /// List enabled schedules whose `next_trigger_at` is in the past or now.
    fn list_due_schedules(&self) -> StoreFuture<'_, Vec<Schedule>>;
}
