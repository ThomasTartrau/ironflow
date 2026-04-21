//! The [`AuditLogStore`] trait -- async storage abstraction for audit log entries.
//!
//! Persists every domain event for compliance and post-mortem debugging.

use crate::entities::{AuditLogEntry, AuditLogFilter, NewAuditLogEntry, Page};
use crate::store::StoreFuture;

/// Async storage abstraction for audit log entries.
///
/// All methods return a [`StoreFuture`] (boxed future) for object safety,
/// allowing the store to be used as `Arc<dyn AuditLogStore>`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_store::prelude::*;
/// use serde_json::json;
/// use uuid::Uuid;
///
/// # async fn example() -> Result<(), ironflow_store::error::StoreError> {
/// let store = InMemoryStore::new();
///
/// let entry = store.append_audit_log(NewAuditLogEntry {
///     event_type: EventKind::RunCreated,
///     payload: json!({"run_id": "abc"}),
///     run_id: Some(Uuid::now_v7()),
///     step_id: None,
///     user_id: None,
/// }).await?;
///
/// assert_eq!(entry.event_type, EventKind::RunCreated);
/// # Ok(())
/// # }
/// ```
pub trait AuditLogStore: Send + Sync {
    /// Persist a new audit log entry.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] if the backing store fails.
    fn append_audit_log(&self, entry: NewAuditLogEntry) -> StoreFuture<'_, AuditLogEntry>;

    /// List audit log entries matching the given filter, with pagination.
    ///
    /// Results are ordered by `created_at` descending (newest first).
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] if the backing store fails.
    fn list_audit_logs(
        &self,
        filter: AuditLogFilter,
        page: u32,
        per_page: u32,
    ) -> StoreFuture<'_, Page<AuditLogEntry>>;
}
