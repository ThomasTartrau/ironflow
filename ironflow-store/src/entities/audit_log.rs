//! Audit log entry entity for compliance and post-mortem debugging.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use super::EventKind;

/// A persisted audit log entry capturing a domain event.
///
/// Each entry records the full event payload as JSON alongside
/// denormalized contextual IDs for efficient filtering.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::{AuditLogEntry, EventKind};
/// use uuid::Uuid;
/// use chrono::Utc;
/// use serde_json::json;
///
/// let entry = AuditLogEntry {
///     id: Uuid::now_v7(),
///     event_type: EventKind::RunStatusChanged,
///     payload: json!({"run_id": "..."}),
///     run_id: Some(Uuid::now_v7()),
///     step_id: None,
///     user_id: None,
///     created_at: Utc::now(),
/// };
/// assert_eq!(entry.event_type, EventKind::RunStatusChanged);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AuditLogEntry {
    /// Unique entry ID (UUID v7).
    pub id: Uuid,
    /// Event type.
    pub event_type: EventKind,
    /// Full serialized event payload.
    pub payload: Value,
    /// Associated run ID, if the event is run-scoped.
    pub run_id: Option<Uuid>,
    /// Associated step ID, if the event is step-scoped.
    pub step_id: Option<Uuid>,
    /// Associated user ID, if the event is user-scoped.
    pub user_id: Option<Uuid>,
    /// When the event was recorded.
    pub created_at: DateTime<Utc>,
}

/// Parameters for creating a new audit log entry.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::{NewAuditLogEntry, EventKind};
/// use uuid::Uuid;
/// use serde_json::json;
///
/// let new_entry = NewAuditLogEntry {
///     event_type: EventKind::RunCreated,
///     payload: json!({"run_id": "abc"}),
///     run_id: Some(Uuid::now_v7()),
///     step_id: None,
///     user_id: None,
/// };
/// ```
#[derive(Debug, Clone)]
pub struct NewAuditLogEntry {
    /// Event type.
    pub event_type: EventKind,
    /// Full serialized event payload.
    pub payload: Value,
    /// Associated run ID.
    pub run_id: Option<Uuid>,
    /// Associated step ID.
    pub step_id: Option<Uuid>,
    /// Associated user ID.
    pub user_id: Option<Uuid>,
}

/// Filter criteria for listing audit log entries.
///
/// All fields are optional. When `None`, no filtering is applied
/// for that dimension.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::{AuditLogFilter, EventKind};
///
/// let filter = AuditLogFilter {
///     event_type: Some(EventKind::RunFailed),
///     ..AuditLogFilter::default()
/// };
/// ```
#[derive(Debug, Clone, Default)]
pub struct AuditLogFilter {
    /// Filter by event type.
    pub event_type: Option<EventKind>,
    /// Filter by run ID.
    pub run_id: Option<Uuid>,
    /// Filter entries created at or after this timestamp.
    pub from: Option<DateTime<Utc>>,
    /// Filter entries created at or before this timestamp.
    pub to: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn audit_log_entry_serde_roundtrip() {
        let entry = AuditLogEntry {
            id: Uuid::now_v7(),
            event_type: EventKind::RunStatusChanged,
            payload: json!({"run_id": "abc", "from": "pending", "to": "running"}),
            run_id: Some(Uuid::now_v7()),
            step_id: None,
            user_id: None,
            created_at: Utc::now(),
        };

        let json = serde_json::to_string(&entry).expect("serialize");
        let back: AuditLogEntry = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(back.event_type, EventKind::RunStatusChanged);
        assert_eq!(back.id, entry.id);
        assert!(json.contains("run_status_changed"));
    }

    #[test]
    fn new_audit_log_entry_creation() {
        let new_entry = NewAuditLogEntry {
            event_type: EventKind::StepFailed,
            payload: json!({"step_id": "xyz"}),
            run_id: Some(Uuid::now_v7()),
            step_id: Some(Uuid::now_v7()),
            user_id: None,
        };

        assert_eq!(new_entry.event_type, EventKind::StepFailed);
        assert!(new_entry.run_id.is_some());
        assert!(new_entry.step_id.is_some());
        assert!(new_entry.user_id.is_none());
    }

    #[test]
    fn audit_log_filter_default_is_empty() {
        let filter = AuditLogFilter::default();
        assert!(filter.event_type.is_none());
        assert!(filter.run_id.is_none());
        assert!(filter.from.is_none());
        assert!(filter.to.is_none());
    }

    #[test]
    fn audit_log_filter_with_all_fields() {
        let now = Utc::now();
        let filter = AuditLogFilter {
            event_type: Some(EventKind::RunCreated),
            run_id: Some(Uuid::now_v7()),
            from: Some(now),
            to: Some(now),
        };

        assert_eq!(filter.event_type, Some(EventKind::RunCreated));
        assert!(filter.run_id.is_some());
        assert!(filter.from.is_some());
        assert!(filter.to.is_some());
    }
}
