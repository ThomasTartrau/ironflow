//! [`Run`] entity and related request/update types.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use super::{FsmState, RunStatus, TriggerKind};

/// A workflow execution record.
///
/// Represents a single invocation of a workflow, tracking its status through
/// the [`RunStatus`] FSM (SQL-side via [`lib_fsm`](crate::postgres::helpers::lib_fsm)),
/// aggregated metrics, and timestamps.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::Run;
///
/// // Runs are created by RunStore::create_run, not directly.
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Run {
    /// Unique identifier (UUIDv7, sortable by creation time).
    pub id: Uuid,
    /// Name of the workflow that was executed.
    pub workflow_name: String,
    /// Current FSM status — embeds state + state_machine_id for SQL-side transitions.
    pub status: FsmState<RunStatus>,
    /// How this run was triggered.
    pub trigger: TriggerKind,
    /// Trigger-specific payload (e.g. webhook body).
    pub payload: Value,
    /// Error message if the run failed.
    pub error: Option<String>,
    /// Number of times this run has been retried.
    pub retry_count: u32,
    /// Maximum number of retries allowed.
    pub max_retries: u32,
    /// Aggregated cost across all agent steps, in USD.
    pub cost_usd: Decimal,
    /// Aggregated wall-clock duration across all steps, in milliseconds.
    pub duration_ms: u64,
    /// When the run was created (enqueued).
    pub created_at: DateTime<Utc>,
    /// When the run record was last updated.
    pub updated_at: DateTime<Utc>,
    /// When execution started (transitioned to Running).
    pub started_at: Option<DateTime<Utc>>,
    /// When execution finished (transitioned to a terminal state).
    pub completed_at: Option<DateTime<Utc>>,
}

/// Request to create a new run.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::{NewRun, TriggerKind};
/// use serde_json::json;
///
/// let req = NewRun {
///     workflow_name: "deploy".to_string(),
///     trigger: TriggerKind::Manual,
///     payload: json!({}),
///     max_retries: 3,
/// };
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewRun {
    /// Workflow name.
    pub workflow_name: String,
    /// How the run was triggered.
    pub trigger: TriggerKind,
    /// Trigger-specific payload.
    pub payload: Value,
    /// Maximum retry attempts.
    pub max_retries: u32,
}

/// Filters for listing runs.
///
/// All fields are optional; `None` means "no filter" for that field.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::{RunFilter, RunStatus};
///
/// let filter = RunFilter {
///     workflow_name: Some("deploy".to_string()),
///     status: Some(RunStatus::Completed),
///     ..RunFilter::default()
/// };
/// ```
#[derive(Debug, Clone, Default)]
pub struct RunFilter {
    /// Filter by workflow name (exact match).
    pub workflow_name: Option<String>,
    /// Filter by run status.
    pub status: Option<RunStatus>,
    /// Only include runs created after this timestamp.
    pub created_after: Option<DateTime<Utc>>,
    /// Only include runs created before this timestamp.
    pub created_before: Option<DateTime<Utc>>,
    /// When `Some(true)`, only include runs that have at least one step.
    /// When `Some(false)`, only include runs with no steps.
    /// When `None`, no filtering on steps.
    pub has_steps: Option<bool>,
}

/// Partial update for a run.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::{RunUpdate, RunStatus};
///
/// let update = RunUpdate {
///     status: Some(RunStatus::Completed),
///     ..RunUpdate::default()
/// };
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RunUpdate {
    /// New status.
    pub status: Option<RunStatus>,
    /// Error message.
    pub error: Option<String>,
    /// Increment retry count.
    pub increment_retry: bool,
    /// Aggregated cost.
    pub cost_usd: Option<Decimal>,
    /// Aggregated duration.
    pub duration_ms: Option<u64>,
    /// When execution started.
    pub started_at: Option<DateTime<Utc>>,
    /// When execution completed.
    pub completed_at: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn newrun_serde_roundtrip() {
        let new_run = NewRun {
            workflow_name: "deploy".to_string(),
            trigger: TriggerKind::Manual,
            payload: json!({"key": "value"}),
            max_retries: 3,
        };

        let json = serde_json::to_string(&new_run).expect("serialize");
        let back: NewRun = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.workflow_name, new_run.workflow_name);
        assert_eq!(back.trigger, new_run.trigger);
        assert_eq!(back.payload, new_run.payload);
        assert_eq!(back.max_retries, new_run.max_retries);
    }

    #[test]
    fn run_serde_preserves_all_fields() {
        use crate::entities::FsmState;
        use chrono::Utc;
        use uuid::Uuid;

        let now = Utc::now();
        let run = Run {
            id: Uuid::now_v7(),
            workflow_name: "test-wf".to_string(),
            status: FsmState::new(RunStatus::Running, Uuid::now_v7()),
            trigger: TriggerKind::Webhook {
                path: "/hooks/test".to_string(),
            },
            payload: json!({"data": 123}),
            error: Some("test error".to_string()),
            retry_count: 2,
            max_retries: 5,
            cost_usd: Decimal::new(1234, 2),
            duration_ms: 5000,
            created_at: now,
            updated_at: now,
            started_at: Some(now),
            completed_at: Some(now),
        };

        let json = serde_json::to_string(&run).expect("serialize");
        let back: Run = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(back.id, run.id);
        assert_eq!(back.workflow_name, run.workflow_name);
        assert_eq!(back.status.state, run.status.state);
        assert_eq!(back.trigger, run.trigger);
        assert_eq!(back.payload, run.payload);
        assert_eq!(back.error, run.error);
        assert_eq!(back.retry_count, run.retry_count);
        assert_eq!(back.max_retries, run.max_retries);
        assert_eq!(back.cost_usd, run.cost_usd);
        assert_eq!(back.duration_ms, run.duration_ms);
        assert_eq!(back.started_at, run.started_at);
        assert_eq!(back.completed_at, run.completed_at);
    }

    #[test]
    fn runupdate_serde_roundtrip() {
        let update = RunUpdate {
            status: Some(RunStatus::Completed),
            error: Some("test error".to_string()),
            increment_retry: true,
            cost_usd: Some(Decimal::new(5000, 2)),
            duration_ms: Some(3000),
            started_at: None,
            completed_at: None,
        };

        let json = serde_json::to_string(&update).expect("serialize");
        let back: RunUpdate = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(back.status, update.status);
        assert_eq!(back.error, update.error);
        assert_eq!(back.increment_retry, update.increment_retry);
        assert_eq!(back.cost_usd, update.cost_usd);
        assert_eq!(back.duration_ms, update.duration_ms);
    }

    #[test]
    fn runfilter_default_is_no_filters() {
        let filter = RunFilter::default();
        assert!(filter.workflow_name.is_none());
        assert!(filter.status.is_none());
        assert!(filter.created_after.is_none());
        assert!(filter.created_before.is_none());
    }

    #[test]
    fn runfilter_with_multiple_criteria() {
        let filter = RunFilter {
            workflow_name: Some("deploy".to_string()),
            status: Some(RunStatus::Running),
            ..RunFilter::default()
        };

        assert_eq!(filter.workflow_name, Some("deploy".to_string()));
        assert_eq!(filter.status, Some(RunStatus::Running));
        assert!(filter.created_after.is_none());
        assert!(filter.created_before.is_none());
    }
}
