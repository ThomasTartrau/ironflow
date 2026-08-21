//! The [`LogStore`] trait -- async storage abstraction for run/step log lines.
//!
//! Persists log output so it can be retrieved after the run finishes,
//! independently of whether an SSE client was connected at the time.

use uuid::Uuid;

use crate::entities::{LogEntry, LogFilter, NewLogEntries};
use crate::store::StoreFuture;

/// Async storage abstraction for run/step log entries.
///
/// All methods return a [`StoreFuture`] (boxed future) for object safety,
/// allowing the store to be used as `Arc<dyn LogStore>`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_store::prelude::*;
/// use uuid::Uuid;
///
/// # async fn example() -> Result<(), ironflow_store::error::StoreError> {
/// let store = InMemoryStore::new();
///
/// store.append_logs(NewLogEntries {
///     run_id: Uuid::now_v7(),
///     step_id: Uuid::now_v7(),
///     step_name: "build".to_string(),
///     stream: LogStream::Stdout,
///     lines: vec!["Compiling ironflow v0.1.0".to_string()],
/// }).await?;
/// # Ok(())
/// # }
/// ```
pub trait LogStore: Send + Sync {
    /// Persist a batch of log lines.
    ///
    /// All lines share the same run, step, and stream. Each line gets
    /// its own [`LogEntry`] with a unique UUID v7 identifier.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] if the backing store fails.
    fn append_logs(&self, entries: NewLogEntries) -> StoreFuture<'_, ()>;

    /// Retrieve log entries for a run with cursor-based pagination.
    ///
    /// Results are ordered by `id` ascending (time-ordered via UUID v7).
    /// Pass the last entry's `id` as `cursor` to fetch the next page.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] if the backing store fails.
    fn get_logs(
        &self,
        run_id: Uuid,
        filter: LogFilter,
        cursor: Option<Uuid>,
        limit: u32,
    ) -> StoreFuture<'_, Vec<LogEntry>>;
}
