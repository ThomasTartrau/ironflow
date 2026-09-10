//! Schedule request and response DTOs.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;
use validator::Validate;

use ironflow_store::entities::{Schedule, ScheduleSource};

/// Schedule list/detail response.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Serialize)]
pub struct ScheduleResponse {
    /// Schedule ID.
    pub id: Uuid,
    /// Name of the workflow to trigger.
    pub workflow_name: String,
    /// Cron expression.
    pub cron_expression: String,
    /// JSON payload passed to the workflow.
    pub inputs: Value,
    /// Where this schedule was created (`handler` or `api`).
    pub source: ScheduleSource,
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

impl From<Schedule> for ScheduleResponse {
    fn from(s: Schedule) -> Self {
        Self {
            id: s.id,
            workflow_name: s.workflow_name,
            cron_expression: s.cron_expression,
            inputs: s.inputs,
            source: s.source,
            disabled_at: s.disabled_at,
            last_triggered_at: s.last_triggered_at,
            next_trigger_at: s.next_trigger_at,
            created_by_user_id: s.created_by_user_id,
            created_at: s.created_at,
            updated_at: s.updated_at,
        }
    }
}

/// Create schedule request body.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Deserialize, Validate)]
pub struct CreateScheduleRequest {
    /// Name of the workflow to trigger.
    #[validate(length(min = 1, message = "workflow_name must not be empty"))]
    pub workflow_name: String,
    /// Cron expression (6-field format, e.g. `"0 */5 * * * *"`).
    #[validate(length(min = 1, message = "cron_expression must not be empty"))]
    pub cron_expression: String,
    /// JSON payload for the workflow. Defaults to `{}`.
    #[serde(default = "default_inputs")]
    pub inputs: Value,
}

fn default_inputs() -> Value {
    Value::Object(serde_json::Map::new())
}

/// Update schedule request body. All fields optional.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Deserialize)]
pub struct UpdateScheduleRequest {
    /// New cron expression.
    pub cron_expression: Option<String>,
    /// New inputs payload.
    pub inputs: Option<Value>,
}
