//! Schedule entity for periodic workflow execution.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// A persisted schedule that triggers a workflow on a cron expression.
///
/// A schedule is active when `disabled_at` is `None`. Setting `disabled_at`
/// to a timestamp pauses it.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::Schedule;
/// use chrono::Utc;
/// use serde_json::json;
/// use uuid::Uuid;
///
/// let schedule = Schedule {
///     id: Uuid::now_v7(),
///     workflow_name: "deploy".to_string(),
///     cron_expression: "0 0 * * * *".to_string(),
///     inputs: json!({}),
///     disabled_at: None,
///     last_triggered_at: None,
///     next_trigger_at: Some(Utc::now()),
///     created_by_user_id: Uuid::now_v7(),
///     created_at: Utc::now(),
///     updated_at: Utc::now(),
/// };
/// assert!(schedule.is_active());
/// ```
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schedule {
    /// Unique schedule ID (UUID v7).
    pub id: Uuid,
    /// Name of the workflow to trigger.
    pub workflow_name: String,
    /// Cron expression (6-field format, e.g. `"0 */5 * * * *"`).
    pub cron_expression: String,
    /// JSON payload passed to the workflow on each trigger.
    pub inputs: Value,
    /// When the schedule was disabled. `None` means active.
    pub disabled_at: Option<DateTime<Utc>>,
    /// When the schedule last created a run.
    pub last_triggered_at: Option<DateTime<Utc>>,
    /// When the schedule will next fire.
    pub next_trigger_at: Option<DateTime<Utc>>,
    /// User who created the schedule.
    pub created_by_user_id: Uuid,
    /// When the schedule was created.
    pub created_at: DateTime<Utc>,
    /// When the schedule was last updated.
    pub updated_at: DateTime<Utc>,
}

impl Schedule {
    /// Whether the schedule is currently active (not disabled).
    pub fn is_active(&self) -> bool {
        self.disabled_at.is_none()
    }
}

/// Parameters for creating a new schedule.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::NewSchedule;
/// use serde_json::json;
/// use uuid::Uuid;
/// use chrono::Utc;
///
/// let new = NewSchedule {
///     workflow_name: "deploy".to_string(),
///     cron_expression: "0 0 * * * *".to_string(),
///     inputs: json!({"env": "prod"}),
///     created_by_user_id: Uuid::now_v7(),
///     next_trigger_at: Some(Utc::now()),
/// };
/// assert_eq!(new.workflow_name, "deploy");
/// ```
#[derive(Debug, Clone)]
pub struct NewSchedule {
    /// Name of the workflow to trigger.
    pub workflow_name: String,
    /// Cron expression (6-field format).
    pub cron_expression: String,
    /// JSON payload for the workflow.
    pub inputs: Value,
    /// User who creates the schedule.
    pub created_by_user_id: Uuid,
    /// Pre-computed next trigger time.
    pub next_trigger_at: Option<DateTime<Utc>>,
}

/// Updatable fields on a schedule.
///
/// Only `Some` fields are applied; `None` means "leave unchanged".
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::ScheduleUpdate;
/// use serde_json::json;
///
/// let update = ScheduleUpdate {
///     cron_expression: Some("0 30 * * * *".to_string()),
///     inputs: Some(json!({"env": "staging"})),
///     disabled_at: None,
///     next_trigger_at: None,
///     last_triggered_at: None,
/// };
/// assert!(update.disabled_at.is_none());
/// ```
#[derive(Debug, Clone, Default)]
pub struct ScheduleUpdate {
    /// New cron expression.
    pub cron_expression: Option<String>,
    /// New inputs payload.
    pub inputs: Option<Value>,
    /// Set or clear disabled_at. `Some(Some(ts))` disables, `Some(None)` re-enables.
    pub disabled_at: Option<Option<DateTime<Utc>>>,
    /// Updated next trigger time.
    pub next_trigger_at: Option<Option<DateTime<Utc>>>,
    /// Updated last triggered time.
    pub last_triggered_at: Option<Option<DateTime<Utc>>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn schedule_serde_roundtrip() {
        let schedule = Schedule {
            id: Uuid::now_v7(),
            workflow_name: "deploy".to_string(),
            cron_expression: "0 0 * * * *".to_string(),
            inputs: json!({"env": "prod"}),
            disabled_at: None,
            last_triggered_at: None,
            next_trigger_at: Some(Utc::now()),
            created_by_user_id: Uuid::now_v7(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let json_str = serde_json::to_string(&schedule).expect("serialize");
        let back: Schedule = serde_json::from_str(&json_str).expect("deserialize");
        assert_eq!(schedule.id, back.id);
        assert_eq!(schedule.workflow_name, back.workflow_name);
        assert!(back.is_active());
    }

    #[test]
    fn disabled_schedule_is_not_active() {
        let schedule = Schedule {
            id: Uuid::now_v7(),
            workflow_name: "deploy".to_string(),
            cron_expression: "0 0 * * * *".to_string(),
            inputs: json!({}),
            disabled_at: Some(Utc::now()),
            last_triggered_at: None,
            next_trigger_at: None,
            created_by_user_id: Uuid::now_v7(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        assert!(!schedule.is_active());
    }

    #[test]
    fn schedule_update_defaults_to_none() {
        let update = ScheduleUpdate::default();
        assert!(update.cron_expression.is_none());
        assert!(update.inputs.is_none());
        assert!(update.disabled_at.is_none());
        assert!(update.next_trigger_at.is_none());
        assert!(update.last_triggered_at.is_none());
    }
}
