//! Schedule entity for periodic workflow execution.

use std::collections::HashMap;

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use strum::{Display, EnumString, IntoStaticStr};
use uuid::Uuid;

use super::{NewRun, RunActor, RunCreation, TriggerKind};

/// Where a schedule was created.
///
/// `Handler` schedules are declared in code via [`WorkflowHandler::schedule()`]
/// and synced to the database at startup. `Api` schedules are created by users
/// through the REST API, CLI, or dashboard.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::ScheduleSource;
///
/// let source = ScheduleSource::Handler;
/// assert_eq!(source.as_str(), "handler");
///
/// let parsed: ScheduleSource = "api".parse().unwrap();
/// assert_eq!(parsed, ScheduleSource::Api);
/// ```
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, EnumString, IntoStaticStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum ScheduleSource {
    /// Declared in code via `WorkflowHandler::schedule()`.
    Handler,
    /// Created via the REST API.
    Api,
}

impl ScheduleSource {
    /// String representation used in the database.
    pub fn as_str(&self) -> &'static str {
        self.into()
    }
}

/// A persisted schedule that triggers a workflow on a cron expression.
///
/// A schedule is active when `disabled_at` is `None`. Setting `disabled_at`
/// to a timestamp pauses it.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::{Schedule, ScheduleSource};
/// use chrono::Utc;
/// use serde_json::json;
/// use uuid::Uuid;
///
/// let schedule = Schedule {
///     id: Uuid::now_v7(),
///     workflow_name: "deploy".to_string(),
///     cron_expression: "0 0 * * * *".to_string(),
///     inputs: json!({}),
///     source: ScheduleSource::Api,
///     disabled_at: None,
///     last_triggered_at: None,
///     next_trigger_at: Some(Utc::now()),
///     last_error: None,
///     priority: 0,
///     created_by_user_id: Some(Uuid::now_v7()),
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
    /// Where this schedule was created.
    pub source: ScheduleSource,
    /// When the schedule was disabled. `None` means active.
    pub disabled_at: Option<DateTime<Utc>>,
    /// When the schedule last created a run.
    pub last_triggered_at: Option<DateTime<Utc>>,
    /// When the schedule will next fire. Always set on an active schedule.
    pub next_trigger_at: Option<DateTime<Utc>>,
    /// Why Ironflow disabled the schedule on its own, e.g. a cron expression
    /// whose next occurrence cannot be computed. `None` for a schedule paused
    /// by a user or never disabled.
    #[serde(default)]
    pub last_error: Option<String>,
    /// User who created the schedule. `None` for handler-declared schedules,
    /// which have no human author.
    pub created_by_user_id: Option<Uuid>,
    /// When the schedule was created.
    pub created_at: DateTime<Utc>,
    /// When the schedule was last updated.
    pub updated_at: DateTime<Utc>,
    /// Queue priority given to every run this schedule creates, between
    /// [`MIN_PRIORITY`](super::MIN_PRIORITY) and [`MAX_PRIORITY`](super::MAX_PRIORITY).
    ///
    /// Defaults to `0` when absent from the payload.
    #[serde(default)]
    pub priority: i16,
}

impl Schedule {
    /// Whether the schedule is currently active (not disabled).
    pub fn is_active(&self) -> bool {
        self.disabled_at.is_none()
    }

    /// Idempotency key of the run created for one occurrence of this schedule:
    /// `schedule:<id>:<occurrence in RFC 3339>`.
    ///
    /// Two firings of the same occurrence (several servers, a retry after a
    /// crash) share the key, so they never create two runs.
    ///
    /// # Examples
    ///
    /// ```
    /// use chrono::{TimeZone, Utc};
    /// use ironflow_store::entities::Schedule;
    /// use uuid::Uuid;
    ///
    /// let id = Uuid::nil();
    /// let at = Utc.with_ymd_and_hms(2026, 10, 6, 8, 0, 0).unwrap();
    /// assert_eq!(
    ///     Schedule::occurrence_key(id, at),
    ///     "schedule:00000000-0000-0000-0000-000000000000:2026-10-06T08:00:00Z",
    /// );
    /// ```
    pub fn occurrence_key(id: Uuid, occurrence: DateTime<Utc>) -> String {
        format!(
            "schedule:{id}:{}",
            occurrence.to_rfc3339_opts(SecondsFormat::AutoSi, true)
        )
    }

    /// Build the run this schedule creates: its workflow, its inputs as
    /// payload, and a [`TriggerKind::Cron`] trigger.
    ///
    /// The run carries no idempotency key: the store sets one when it fires
    /// an occurrence (see [`Schedule::occurrence_key`]).
    ///
    /// # Examples
    ///
    /// ```
    /// use chrono::Utc;
    /// use ironflow_store::entities::{Schedule, ScheduleSource, TriggerKind};
    /// use serde_json::json;
    /// use uuid::Uuid;
    ///
    /// let schedule = Schedule {
    ///     id: Uuid::now_v7(),
    ///     workflow_name: "deploy".to_string(),
    ///     cron_expression: "0 0 * * * *".to_string(),
    ///     inputs: json!({"env": "prod"}),
    ///     source: ScheduleSource::Api,
    ///     disabled_at: None,
    ///     last_triggered_at: None,
    ///     next_trigger_at: Some(Utc::now()),
    ///     last_error: None,
    ///     priority: 0,
    ///     created_by_user_id: None,
    ///     created_at: Utc::now(),
    ///     updated_at: Utc::now(),
    /// };
    /// let run = schedule.new_run(None);
    /// assert_eq!(run.workflow_name, "deploy");
    /// assert_eq!(run.payload, json!({"env": "prod"}));
    /// assert!(matches!(run.trigger, TriggerKind::Cron { .. }));
    /// ```
    pub fn new_run(&self, created_by: Option<RunActor>) -> NewRun {
        NewRun {
            workflow_name: self.workflow_name.clone(),
            trigger: TriggerKind::Cron {
                schedule: self.cron_expression.clone(),
            },
            payload: self.inputs.clone(),
            max_retries: 0,
            handler_version: None,
            labels: HashMap::new(),
            scheduled_at: None,
            created_by,
            idempotency_key: None,
            concurrency_key: None,
            priority: self.priority,
            concurrency_limits: Vec::new(),
            max_cost_usd: None,
            worker_tags: Vec::new(),
        }
    }
}

/// What happens to a schedule after it fires an occurrence.
///
/// Computed by the caller from the cron expression, applied by
/// [`ScheduleStore::fire_due_schedule`](crate::schedule_store::ScheduleStore::fire_due_schedule)
/// in the same transaction as the run creation.
///
/// # Examples
///
/// ```
/// use chrono::Utc;
/// use ironflow_store::entities::ScheduleNext;
///
/// let next = ScheduleNext::At(Utc::now());
/// assert!(matches!(next, ScheduleNext::At(_)));
///
/// let stop = ScheduleNext::Disable { error: "no next occurrence".to_string() };
/// assert!(matches!(stop, ScheduleNext::Disable { .. }));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScheduleNext {
    /// Fire again at this time.
    At(DateTime<Utc>),
    /// Disable the schedule: its next occurrence cannot be computed. The
    /// error is stored in [`Schedule::last_error`].
    Disable {
        /// Why the next occurrence cannot be computed.
        error: String,
    },
}

/// Result of firing one occurrence of a schedule.
///
/// # Examples
///
/// ```no_run
/// use chrono::Utc;
/// use ironflow_store::entities::ScheduleNext;
/// use ironflow_store::memory::InMemoryStore;
/// use ironflow_store::schedule_store::ScheduleStore;
/// use uuid::Uuid;
///
/// # async fn example(id: Uuid, occurrence: chrono::DateTime<Utc>) -> Result<(), ironflow_store::error::StoreError> {
/// let store = InMemoryStore::new();
/// if let Some(firing) = store
///     .fire_due_schedule(id, occurrence, ScheduleNext::At(Utc::now()))
///     .await?
/// {
///     println!("run {} created", firing.run.run().id);
/// }
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct ScheduleFiring {
    /// The schedule after the firing: next occurrence set, or disabled.
    pub schedule: Schedule,
    /// The run of the occurrence. [`RunCreation::Existing`] when a run with
    /// the same occurrence key already existed.
    pub run: RunCreation,
}

/// Parameters for creating a new schedule.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::{NewSchedule, ScheduleSource};
/// use serde_json::json;
/// use uuid::Uuid;
/// use chrono::Utc;
///
/// let new = NewSchedule {
///     workflow_name: "deploy".to_string(),
///     cron_expression: "0 0 * * * *".to_string(),
///     inputs: json!({"env": "prod"}),
///     source: ScheduleSource::Api,
///     priority: 0,
///     created_by_user_id: Some(Uuid::now_v7()),
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
    /// Where this schedule originates.
    pub source: ScheduleSource,
    /// User who creates the schedule. `None` for handler-declared schedules,
    /// which have no human author.
    pub created_by_user_id: Option<Uuid>,
    /// Pre-computed next trigger time.
    pub next_trigger_at: Option<DateTime<Utc>>,
    /// Queue priority given to every run the schedule creates, between
    /// [`MIN_PRIORITY`](super::MIN_PRIORITY) and [`MAX_PRIORITY`](super::MAX_PRIORITY).
    pub priority: i16,
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
///     priority: None,
///     last_error: None,
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
    /// Set or clear [`Schedule::last_error`]. `Some(None)` clears it.
    pub last_error: Option<Option<String>>,
    /// New queue priority for the runs the schedule creates.
    pub priority: Option<i16>,
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
            source: ScheduleSource::Api,
            disabled_at: None,
            last_triggered_at: None,
            next_trigger_at: Some(Utc::now()),
            last_error: None,
            priority: -5,
            created_by_user_id: Some(Uuid::now_v7()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let json_str = serde_json::to_string(&schedule).expect("serialize");
        let back: Schedule = serde_json::from_str(&json_str).expect("deserialize");
        assert_eq!(schedule.id, back.id);
        assert_eq!(schedule.workflow_name, back.workflow_name);
        assert_eq!(back.priority, -5);
        assert!(back.is_active());
    }

    #[test]
    fn schedule_new_run_carries_the_schedule_priority() {
        let schedule = Schedule {
            id: Uuid::now_v7(),
            workflow_name: "deploy".to_string(),
            cron_expression: "0 0 * * * *".to_string(),
            inputs: json!({}),
            source: ScheduleSource::Api,
            disabled_at: None,
            last_triggered_at: None,
            next_trigger_at: Some(Utc::now()),
            last_error: None,
            priority: 42,
            created_by_user_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        assert_eq!(schedule.new_run(None).priority, 42);
    }

    #[test]
    fn disabled_schedule_is_not_active() {
        let schedule = Schedule {
            id: Uuid::now_v7(),
            workflow_name: "deploy".to_string(),
            cron_expression: "0 0 * * * *".to_string(),
            inputs: json!({}),
            source: ScheduleSource::Api,
            disabled_at: Some(Utc::now()),
            last_triggered_at: None,
            next_trigger_at: None,
            last_error: None,
            priority: 0,
            created_by_user_id: Some(Uuid::now_v7()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        assert!(!schedule.is_active());
    }

    #[test]
    fn schedule_source_roundtrip() {
        assert_eq!(ScheduleSource::Handler.as_str(), "handler");
        assert_eq!(ScheduleSource::Api.as_str(), "api");

        let parsed: ScheduleSource = "handler".parse().unwrap();
        assert_eq!(parsed, ScheduleSource::Handler);

        let parsed: ScheduleSource = "api".parse().unwrap();
        assert_eq!(parsed, ScheduleSource::Api);

        assert!("unknown".parse::<ScheduleSource>().is_err());
    }

    #[test]
    fn schedule_update_defaults_to_none() {
        let update = ScheduleUpdate::default();
        assert!(update.cron_expression.is_none());
        assert!(update.inputs.is_none());
        assert!(update.disabled_at.is_none());
        assert!(update.next_trigger_at.is_none());
        assert!(update.last_triggered_at.is_none());
        assert!(update.last_error.is_none());
        assert!(update.priority.is_none());
    }
}
