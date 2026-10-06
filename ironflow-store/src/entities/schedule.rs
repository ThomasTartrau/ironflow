//! Schedule entity for periodic workflow execution.

use std::collections::HashMap;

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use chrono_tz::Tz;
use strum::{AsRefStr, Display, EnumString, IntoStaticStr};
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

/// What a schedule does with the occurrences it missed while no server was
/// firing it (downtime, a long deploy, a stalled ticker).
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::CatchupPolicy;
///
/// assert_eq!(CatchupPolicy::default(), CatchupPolicy::Latest);
/// assert_eq!(CatchupPolicy::All.as_ref(), "all");
///
/// let parsed: CatchupPolicy = "skip".parse().unwrap();
/// assert_eq!(parsed, CatchupPolicy::Skip);
/// ```
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Display,
    EnumString,
    IntoStaticStr,
    AsRefStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum CatchupPolicy {
    /// Run only the most recent missed occurrence.
    #[default]
    Latest,
    /// Run every missed occurrence, oldest first, up to
    /// [`SchedulePolicy::catchup_max`].
    All,
    /// Run no late occurrence: only an occurrence fired on time runs.
    Skip,
}

/// What a schedule does when an occurrence comes while one of its runs is
/// still active.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::OverlapPolicy;
///
/// assert_eq!(OverlapPolicy::default(), OverlapPolicy::Allow);
/// assert_eq!(OverlapPolicy::Skip.as_ref(), "skip");
///
/// let parsed: OverlapPolicy = "allow".parse().unwrap();
/// assert_eq!(parsed, OverlapPolicy::Allow);
/// ```
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Display,
    EnumString,
    IntoStaticStr,
    AsRefStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum OverlapPolicy {
    /// Create the run anyway: runs of the schedule may run side by side.
    #[default]
    Allow,
    /// Skip the occurrence while a run of the schedule is active. Enforced
    /// with the concurrency key [`Schedule::concurrency_key`].
    Skip,
}

/// Default [`SchedulePolicy::catchup_max`].
pub const DEFAULT_CATCHUP_MAX: u32 = 10;
/// Lowest accepted [`SchedulePolicy::catchup_max`].
pub const MIN_CATCHUP_MAX: u32 = 1;
/// Highest accepted [`SchedulePolicy::catchup_max`].
pub const MAX_CATCHUP_MAX: u32 = 1000;
/// Default [`SchedulePolicy::catchup_window_secs`]: one day.
pub const DEFAULT_CATCHUP_WINDOW_SECS: u32 = 86_400;
/// Lowest accepted [`SchedulePolicy::catchup_window_secs`]: one minute.
pub const MIN_CATCHUP_WINDOW_SECS: u32 = 60;
/// Highest accepted [`SchedulePolicy::catchup_window_secs`]: thirty days.
pub const MAX_CATCHUP_WINDOW_SECS: u32 = 2_592_000;
/// Default [`SchedulePolicy::timezone`].
pub const DEFAULT_TIMEZONE: Tz = Tz::UTC;

/// Catch-up, overlap and timezone policy of a schedule.
///
/// # Examples
///
/// ```
/// use chrono_tz::Tz;
/// use ironflow_store::entities::{CatchupPolicy, OverlapPolicy, SchedulePolicy};
///
/// let policy = SchedulePolicy {
///     catchup: CatchupPolicy::All,
///     overlap: OverlapPolicy::Skip,
///     timezone: Tz::Europe__Paris,
///     ..SchedulePolicy::default()
/// };
/// assert!(policy.validate().is_ok());
///
/// let invalid = SchedulePolicy { catchup_max: 0, ..SchedulePolicy::default() };
/// assert!(invalid.validate().is_err());
/// ```
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchedulePolicy {
    /// What to do with missed occurrences.
    pub catchup: CatchupPolicy,
    /// Most runs created to catch up under [`CatchupPolicy::All`], between
    /// [`MIN_CATCHUP_MAX`] and [`MAX_CATCHUP_MAX`].
    pub catchup_max: u32,
    /// How far back, in seconds, a missed occurrence is still caught up,
    /// between [`MIN_CATCHUP_WINDOW_SECS`] and [`MAX_CATCHUP_WINDOW_SECS`].
    pub catchup_window_secs: u32,
    /// What to do when a run of the schedule is still active.
    pub overlap: OverlapPolicy,
    /// IANA timezone the cron expression is evaluated in, e.g. `Europe/Paris`.
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "Europe/Paris"))]
    pub timezone: Tz,
}

impl Default for SchedulePolicy {
    fn default() -> Self {
        Self {
            catchup: CatchupPolicy::default(),
            catchup_max: DEFAULT_CATCHUP_MAX,
            catchup_window_secs: DEFAULT_CATCHUP_WINDOW_SECS,
            overlap: OverlapPolicy::default(),
            timezone: DEFAULT_TIMEZONE,
        }
    }
}

impl SchedulePolicy {
    /// Check the numeric bounds of the policy. The timezone is checked where
    /// the timezone database is available (API and engine).
    ///
    /// # Errors
    ///
    /// Returns a message naming the field out of its range.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_store::entities::SchedulePolicy;
    ///
    /// let policy = SchedulePolicy { catchup_window_secs: 10, ..SchedulePolicy::default() };
    /// assert_eq!(
    ///     policy.validate().unwrap_err(),
    ///     "catchup_window_secs must be between 60 and 2592000",
    /// );
    /// ```
    pub fn validate(&self) -> Result<(), String> {
        if !(MIN_CATCHUP_MAX..=MAX_CATCHUP_MAX).contains(&self.catchup_max) {
            return Err(format!(
                "catchup_max must be between {MIN_CATCHUP_MAX} and {MAX_CATCHUP_MAX}"
            ));
        }
        if !(MIN_CATCHUP_WINDOW_SECS..=MAX_CATCHUP_WINDOW_SECS).contains(&self.catchup_window_secs)
        {
            return Err(format!(
                "catchup_window_secs must be between {MIN_CATCHUP_WINDOW_SECS} and {MAX_CATCHUP_WINDOW_SECS}"
            ));
        }
        Ok(())
    }
}

/// Why an occurrence of a schedule created no run.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::ScheduleMissReason;
///
/// let reason = ScheduleMissReason::Overlap;
/// assert_eq!(reason.to_string(), "overlap");
/// assert_eq!(serde_json::to_string(&reason).unwrap(), "\"overlap\"");
/// ```
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, EnumString, IntoStaticStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum ScheduleMissReason {
    /// Older than the schedule's catch-up window.
    OutsideWindow,
    /// Dropped because [`SchedulePolicy::catchup_max`] runs were already
    /// caught up under [`CatchupPolicy::All`].
    CatchupMax,
    /// Replaced by a more recent occurrence under [`CatchupPolicy::Latest`].
    Superseded,
    /// Late, and not caught up under [`CatchupPolicy::Skip`].
    CatchupSkip,
    /// A run of the schedule was still active under [`OverlapPolicy::Skip`].
    Overlap,
}

/// A persisted schedule that triggers a workflow on a cron expression.
///
/// A schedule is active when `disabled_at` is `None`. Setting `disabled_at`
/// to a timestamp pauses it.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::{Schedule, SchedulePolicy, ScheduleSource};
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
///     policy: SchedulePolicy::default(),
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
    /// Catch-up, overlap and timezone policy. Defaults to
    /// [`SchedulePolicy::default`] when absent from the payload.
    #[serde(default)]
    pub policy: SchedulePolicy,
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

    /// Concurrency key of the runs of a schedule under
    /// [`OverlapPolicy::Skip`]: `schedule:<id>`.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_store::entities::Schedule;
    /// use uuid::Uuid;
    ///
    /// assert_eq!(
    ///     Schedule::concurrency_key(Uuid::nil()),
    ///     "schedule:00000000-0000-0000-0000-000000000000",
    /// );
    /// ```
    pub fn concurrency_key(id: Uuid) -> String {
        format!("schedule:{id}")
    }

    /// Build the run this schedule creates: its workflow, its inputs as
    /// payload, and a [`TriggerKind::Cron`] trigger carrying the schedule id
    /// and the occurrence it covers (`None` for a manual trigger).
    ///
    /// Under [`OverlapPolicy::Skip`] the run carries the concurrency key
    /// [`Schedule::concurrency_key`], so it cannot start while another run
    /// of the schedule is active.
    ///
    /// The run carries no idempotency key: the store sets one when it fires
    /// an occurrence (see [`Schedule::occurrence_key`]).
    ///
    /// # Examples
    ///
    /// ```
    /// use chrono::Utc;
    /// use ironflow_store::entities::{Schedule, SchedulePolicy, ScheduleSource, TriggerKind};
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
    ///     policy: SchedulePolicy::default(),
    ///     created_by_user_id: None,
    ///     created_at: Utc::now(),
    ///     updated_at: Utc::now(),
    /// };
    /// let occurrence = Utc::now();
    /// let run = schedule.new_run(Some(occurrence), None);
    /// assert_eq!(run.workflow_name, "deploy");
    /// assert_eq!(run.payload, json!({"env": "prod"}));
    /// assert!(matches!(
    ///     run.trigger,
    ///     TriggerKind::Cron { scheduled_for: Some(at), .. } if at == occurrence
    /// ));
    /// ```
    pub fn new_run(
        &self,
        scheduled_for: Option<DateTime<Utc>>,
        created_by: Option<RunActor>,
    ) -> NewRun {
        NewRun {
            workflow_name: self.workflow_name.clone(),
            trigger: TriggerKind::Cron {
                schedule: self.cron_expression.clone(),
                schedule_id: Some(self.id),
                scheduled_for,
            },
            payload: self.inputs.clone(),
            max_retries: 0,
            handler_version: None,
            labels: HashMap::new(),
            scheduled_at: None,
            created_by,
            idempotency_key: None,
            concurrency_key: (self.policy.overlap == OverlapPolicy::Skip)
                .then(|| Schedule::concurrency_key(self.id)),
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

/// What one firing of a due schedule writes: the occurrences to create a run
/// for, and what happens to the schedule afterwards.
///
/// Computed by the caller from the schedule's [`SchedulePolicy`], applied by
/// [`ScheduleStore::fire_due_schedule`](crate::schedule_store::ScheduleStore::fire_due_schedule).
///
/// # Examples
///
/// ```
/// use chrono::{TimeDelta, Utc};
/// use ironflow_store::entities::{ScheduleFiringPlan, ScheduleNext};
///
/// let now = Utc::now();
/// let plan = ScheduleFiringPlan {
///     occurrences: vec![now - TimeDelta::hours(1), now],
///     next: ScheduleNext::At(now + TimeDelta::hours(1)),
/// };
/// assert_eq!(plan.occurrences.len(), 2);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleFiringPlan {
    /// Occurrences to create a run for, oldest first. May be empty.
    pub occurrences: Vec<DateTime<Utc>>,
    /// What happens to the schedule after the firing.
    pub next: ScheduleNext,
}

/// The run created, or found, for one occurrence of a schedule.
///
/// # Examples
///
/// ```no_run
/// use ironflow_store::entities::ScheduledRun;
///
/// fn describe(run: &ScheduledRun) -> String {
///     format!("{} covers {}", run.run.run().id, run.occurrence)
/// }
/// ```
#[derive(Debug, Clone)]
pub struct ScheduledRun {
    /// The occurrence the run covers.
    pub occurrence: DateTime<Utc>,
    /// The run. [`RunCreation::Existing`] when a run with the same occurrence
    /// key already existed.
    pub run: RunCreation,
}

/// Result of firing a due schedule.
///
/// # Examples
///
/// ```no_run
/// use chrono::Utc;
/// use ironflow_store::entities::{ScheduleFiringPlan, ScheduleNext};
/// use ironflow_store::memory::InMemoryStore;
/// use ironflow_store::schedule_store::ScheduleStore;
/// use uuid::Uuid;
///
/// # async fn example(id: Uuid, due: chrono::DateTime<Utc>) -> Result<(), ironflow_store::error::StoreError> {
/// let store = InMemoryStore::new();
/// let plan = ScheduleFiringPlan {
///     occurrences: vec![due],
///     next: ScheduleNext::At(Utc::now()),
/// };
/// if let Some(firing) = store.fire_due_schedule(id, due, plan).await? {
///     for scheduled in &firing.runs {
///         println!("run {} created for {}", scheduled.run.run().id, scheduled.occurrence);
///     }
///     println!("{} occurrences skipped as overlapping", firing.overlapped.len());
/// }
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct ScheduleFiring {
    /// The schedule after the firing: next occurrence set, or disabled.
    pub schedule: Schedule,
    /// The runs of the fired occurrences, oldest first.
    pub runs: Vec<ScheduledRun>,
    /// Occurrences refused because a run holding the schedule's concurrency
    /// key was still active ([`OverlapPolicy::Skip`]). Nothing was written
    /// for them.
    pub overlapped: Vec<DateTime<Utc>>,
}

/// Parameters for creating a new schedule.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::{NewSchedule, SchedulePolicy, ScheduleSource};
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
///     policy: SchedulePolicy::default(),
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
    /// Catch-up, overlap and timezone policy.
    pub policy: SchedulePolicy,
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
///     policy: None,
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
    /// New catch-up, overlap and timezone policy.
    pub policy: Option<SchedulePolicy>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schedule(policy: SchedulePolicy) -> Schedule {
        Schedule {
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
            policy,
            created_by_user_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

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
            policy: SchedulePolicy {
                catchup: CatchupPolicy::All,
                timezone: Tz::Europe__Paris,
                ..SchedulePolicy::default()
            },
            created_by_user_id: Some(Uuid::now_v7()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let json_str = serde_json::to_string(&schedule).expect("serialize");
        let back: Schedule = serde_json::from_str(&json_str).expect("deserialize");
        assert_eq!(schedule.id, back.id);
        assert_eq!(schedule.workflow_name, back.workflow_name);
        assert_eq!(back.priority, -5);
        assert_eq!(back.policy, schedule.policy);
        assert!(back.is_active());
    }

    #[test]
    fn schedule_without_policy_deserializes_with_defaults() {
        let mut value = serde_json::to_value(schedule(SchedulePolicy::default())).unwrap();
        value.as_object_mut().unwrap().remove("policy");
        let back: Schedule = serde_json::from_value(value).expect("deserialize");
        assert_eq!(back.policy, SchedulePolicy::default());
    }

    #[test]
    fn schedule_new_run_carries_the_schedule_priority() {
        let schedule = schedule(SchedulePolicy::default());
        assert_eq!(schedule.new_run(None, None).priority, 42);
    }

    #[test]
    fn new_run_carries_schedule_id_and_occurrence() {
        let schedule = schedule(SchedulePolicy::default());
        let occurrence = Utc::now();
        let run = schedule.new_run(Some(occurrence), None);
        assert_eq!(
            run.trigger,
            TriggerKind::Cron {
                schedule: "0 0 * * * *".to_string(),
                schedule_id: Some(schedule.id),
                scheduled_for: Some(occurrence),
            }
        );
        assert!(run.concurrency_key.is_none());

        let manual = schedule.new_run(None, None);
        assert!(matches!(
            manual.trigger,
            TriggerKind::Cron { schedule_id: Some(id), scheduled_for: None, .. } if id == schedule.id
        ));
    }

    #[test]
    fn new_run_with_overlap_skip_sets_the_schedule_concurrency_key() {
        let schedule = schedule(SchedulePolicy {
            overlap: OverlapPolicy::Skip,
            ..SchedulePolicy::default()
        });
        let run = schedule.new_run(Some(Utc::now()), None);
        assert_eq!(
            run.concurrency_key,
            Some(Schedule::concurrency_key(schedule.id))
        );
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
            policy: SchedulePolicy::default(),
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
    fn catchup_and_overlap_policies_roundtrip() {
        for policy in [
            CatchupPolicy::Latest,
            CatchupPolicy::All,
            CatchupPolicy::Skip,
        ] {
            assert_eq!(policy.as_ref().parse::<CatchupPolicy>().unwrap(), policy);
        }
        for policy in [OverlapPolicy::Allow, OverlapPolicy::Skip] {
            assert_eq!(policy.as_ref().parse::<OverlapPolicy>().unwrap(), policy);
        }
        assert!("never".parse::<CatchupPolicy>().is_err());
        assert!("queue".parse::<OverlapPolicy>().is_err());
    }

    #[test]
    fn schedule_policy_defaults() {
        let policy = SchedulePolicy::default();
        assert_eq!(policy.catchup, CatchupPolicy::Latest);
        assert_eq!(policy.catchup_max, DEFAULT_CATCHUP_MAX);
        assert_eq!(policy.catchup_window_secs, DEFAULT_CATCHUP_WINDOW_SECS);
        assert_eq!(policy.overlap, OverlapPolicy::Allow);
        assert_eq!(policy.timezone, Tz::UTC);
        assert!(policy.validate().is_ok());
    }

    #[test]
    fn schedule_policy_validate_checks_bounds() {
        let at = |catchup_max, catchup_window_secs| SchedulePolicy {
            catchup_max,
            catchup_window_secs,
            ..SchedulePolicy::default()
        };
        assert!(
            at(MIN_CATCHUP_MAX, MIN_CATCHUP_WINDOW_SECS)
                .validate()
                .is_ok()
        );
        assert!(
            at(MAX_CATCHUP_MAX, MAX_CATCHUP_WINDOW_SECS)
                .validate()
                .is_ok()
        );
        assert_eq!(
            at(0, 3600).validate().unwrap_err(),
            "catchup_max must be between 1 and 1000"
        );
        assert_eq!(
            at(1001, 3600).validate().unwrap_err(),
            "catchup_max must be between 1 and 1000"
        );
        assert_eq!(
            at(10, 59).validate().unwrap_err(),
            "catchup_window_secs must be between 60 and 2592000"
        );
        assert!(at(10, MAX_CATCHUP_WINDOW_SECS + 1).validate().is_err());
    }

    #[test]
    fn schedule_miss_reason_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&ScheduleMissReason::OutsideWindow).unwrap(),
            "\"outside_window\""
        );
        assert_eq!(ScheduleMissReason::CatchupMax.to_string(), "catchup_max");
        assert_eq!(ScheduleMissReason::CatchupSkip.to_string(), "catchup_skip");
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
        assert!(update.policy.is_none());
    }
}
