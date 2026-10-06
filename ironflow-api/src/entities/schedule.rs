//! Schedule request and response DTOs.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;
use validator::Validate;

use ironflow_store::entities::{CatchupPolicy, OverlapPolicy, Schedule, ScheduleSource};

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
    /// When the schedule will next fire. Always set on an active schedule.
    pub next_trigger_at: Option<DateTime<Utc>>,
    /// Why Ironflow disabled the schedule on its own (e.g. its next trigger
    /// cannot be computed). `None` when paused by a user or never disabled.
    pub last_error: Option<String>,
    /// Queue priority given to every run the schedule creates, from -100 to 100.
    pub priority: i16,
    /// What the schedule does with the occurrences it missed while no server
    /// fired it.
    pub catchup: CatchupPolicy,
    /// Most runs created to catch up under `catchup = all`.
    pub catchup_max: u32,
    /// How far back, in seconds, a missed occurrence is still caught up.
    pub catchup_window_secs: u32,
    /// What the schedule does when an occurrence comes while one of its runs
    /// is still active.
    pub overlap: OverlapPolicy,
    /// IANA timezone the cron expression is evaluated in.
    pub timezone: String,
    /// User who created the schedule. `None` for handler-declared schedules.
    pub created_by_user_id: Option<Uuid>,
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
            last_error: s.last_error,
            priority: s.priority,
            catchup: s.policy.catchup,
            catchup_max: s.policy.catchup_max,
            catchup_window_secs: s.policy.catchup_window_secs,
            overlap: s.policy.overlap,
            timezone: s.policy.timezone,
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
    /// Queue priority given to every run the schedule creates, from -100 to
    /// 100. Defaults to the priority the workflow handler declares.
    ///
    /// A higher priority run is picked first by workers. A running run is
    /// never preempted, and a low priority run is not aged.
    #[serde(default)]
    pub priority: Option<i16>,
    /// What the schedule does with the occurrences it missed while no server
    /// fired it: `latest` (default) runs the most recent one, `all` runs each
    /// of them, `skip` runs none.
    #[serde(default)]
    pub catchup: Option<CatchupPolicy>,
    /// Most runs created to catch up under `catchup = all`, from 1 to 1000.
    /// Defaults to 10.
    #[serde(default)]
    pub catchup_max: Option<u32>,
    /// How far back, in seconds, a missed occurrence is still caught up, from
    /// 60 to 2592000 (30 days). Defaults to 86400 (one day).
    #[serde(default)]
    pub catchup_window_secs: Option<u32>,
    /// What the schedule does when an occurrence comes while one of its runs
    /// is still active: `allow` (default) starts another run, `skip` drops the
    /// occurrence.
    #[serde(default)]
    pub overlap: Option<OverlapPolicy>,
    /// IANA timezone the cron expression is evaluated in, e.g.
    /// `"Europe/Paris"`. Defaults to `"UTC"`.
    #[serde(default)]
    pub timezone: Option<String>,
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
