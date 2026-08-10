//! Conversion helpers between PostgreSQL rows and domain entities.
//!
//! Includes FSM state resolution from lib_fsm schema.

use rust_decimal::Decimal;
use sqlx::Row;

use crate::entities::{FsmState, Run, RunActor, RunStatus, Step, StepKind, StepStatus};
use crate::error::StoreError;

// ---------------------------------------------------------------------------
// Enum ↔ string conversions
// ---------------------------------------------------------------------------

/// Parse a `&str` from the DB into a `RunStatus`.
pub(crate) fn parse_run_status(s: &str) -> Result<RunStatus, StoreError> {
    match s {
        "pending" => Ok(RunStatus::Pending),
        "running" => Ok(RunStatus::Running),
        "completed" => Ok(RunStatus::Completed),
        "failed" => Ok(RunStatus::Failed),
        "retrying" => Ok(RunStatus::Retrying),
        "cancelled" => Ok(RunStatus::Cancelled),
        "awaiting_approval" => Ok(RunStatus::AwaitingApproval),
        other => Err(StoreError::Database(format!("unknown run status: {other}"))),
    }
}

/// Parse a `&str` from the DB into a `StepStatus`.
pub(crate) fn parse_step_status(s: &str) -> Result<StepStatus, StoreError> {
    match s {
        "pending" => Ok(StepStatus::Pending),
        "running" => Ok(StepStatus::Running),
        "completed" => Ok(StepStatus::Completed),
        "failed" => Ok(StepStatus::Failed),
        "skipped" => Ok(StepStatus::Skipped),
        "awaiting_approval" => Ok(StepStatus::AwaitingApproval),
        "rejected" => Ok(StepStatus::Rejected),
        other => Err(StoreError::Database(format!(
            "unknown step status: {other}"
        ))),
    }
}

/// Parse a `&str` from the DB into a `StepKind`.
///
/// Built-in kinds (`shell`, `http`, `agent`, `workflow`) are matched directly.
/// Any other value is treated as a [`StepKind::Custom`] kind.
pub(crate) fn parse_step_kind(s: &str) -> Result<StepKind, StoreError> {
    match s {
        "shell" => Ok(StepKind::Shell),
        "http" => Ok(StepKind::Http),
        "agent" => Ok(StepKind::Agent),
        "workflow" => Ok(StepKind::Workflow),
        "approval" => Ok(StepKind::Approval),
        other => Ok(StepKind::Custom(other.to_string())),
    }
}

/// Convert a `RunStatus` to its DB string representation.
pub(crate) fn run_status_to_db_str(status: &RunStatus) -> &'static str {
    match status {
        RunStatus::Pending => "pending",
        RunStatus::Running => "running",
        RunStatus::Completed => "completed",
        RunStatus::Failed => "failed",
        RunStatus::Retrying => "retrying",
        RunStatus::Cancelled => "cancelled",
        RunStatus::AwaitingApproval => "awaiting_approval",
    }
}

/// Convert a `StepKind` to its DB string representation.
///
/// Built-in kinds return a static `&str`. Custom kinds return the
/// user-provided name as a [`Cow::Owned`].
pub(crate) fn step_kind_to_str(kind: &StepKind) -> std::borrow::Cow<'static, str> {
    match kind {
        StepKind::Shell => std::borrow::Cow::Borrowed("shell"),
        StepKind::Http => std::borrow::Cow::Borrowed("http"),
        StepKind::Agent => std::borrow::Cow::Borrowed("agent"),
        StepKind::Workflow => std::borrow::Cow::Borrowed("workflow"),
        StepKind::Approval => std::borrow::Cow::Borrowed("approval"),
        StepKind::Custom(name) => std::borrow::Cow::Owned(name.clone()),
    }
}

// ---------------------------------------------------------------------------
// Row → entity conversions
// ---------------------------------------------------------------------------

/// Compose the display label for a run author.
///
/// Uses the API key name plus its owner's username when the run was triggered
/// by a key (`"ci-deploy (alice)"`), falling back to the key name alone when the
/// owner no longer exists, and to the username alone for a direct user run.
/// Returns `None` when neither name could be resolved.
fn created_by_label(
    actor: Option<&RunActor>,
    username: Option<String>,
    api_key_name: Option<String>,
) -> Option<String> {
    match actor?.api_key_id() {
        None => username,
        Some(_) => {
            let key_name = api_key_name?;
            Some(match username {
                Some(username) => format!("{key_name} ({username})"),
                None => key_name,
            })
        }
    }
}

/// Convert a row with JOIN to lib_fsm.abstract_state into a Run entity.
///
/// Expected columns: all from ironflow.runs + `ast.name` aliased as
/// `state_name`, plus `created_by_username` and `created_by_api_key_name` from
/// the LEFT JOINs on `iam.users` and `iam.api_keys`.
pub(crate) fn row_to_run(row: &sqlx::postgres::PgRow) -> Result<Run, StoreError> {
    let state_name: &str = row.get("state_name");
    let trigger_json: serde_json::Value = row.get("trigger");
    let cost_usd: Decimal = row.get("cost_usd");
    let state_machine_id = row.get("state_machine__id");
    let labels_json: serde_json::Value = row.get("labels");
    let created_by = RunActor::from_columns(
        row.get("created_by_user_id"),
        row.get("created_by_api_key_id"),
    );
    let created_by_label = created_by_label(
        created_by.as_ref(),
        row.get("created_by_username"),
        row.get("created_by_api_key_name"),
    );

    Ok(Run {
        id: row.get("id"),
        workflow_name: row.get("workflow_name"),
        status: FsmState::new(parse_run_status(state_name)?, state_machine_id),
        trigger: serde_json::from_value(trigger_json)?,
        payload: row.get("payload"),
        error: row.get("error"),
        retry_count: row.get::<i32, _>("retry_count") as u32,
        max_retries: row.get::<i32, _>("max_retries") as u32,
        cost_usd,
        duration_ms: row.get::<i64, _>("duration_ms") as u64,
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        started_at: row.get("started_at"),
        completed_at: row.get("completed_at"),
        handler_version: row.get("handler_version"),
        labels: serde_json::from_value(labels_json).unwrap_or_default(),
        scheduled_at: row.get("scheduled_at"),
        created_by,
        created_by_label,
        idempotency_key: row.get("idempotency_key"),
        max_cost_usd: row.get("max_cost_usd"),
        worker_id: row.get("worker_id"),
        lease_expires_at: row.get("lease_expires_at"),
    })
}

/// Convert a row with JOIN to lib_fsm.abstract_state into a Step entity.
///
/// Expected columns: all from ironflow.steps + `ast.name` aliased as `state_name`.
pub(crate) fn row_to_step(row: &sqlx::postgres::PgRow) -> Result<Step, StoreError> {
    let state_name: &str = row.get("state_name");
    let kind_str: &str = row.get("kind");
    let cost_usd: Decimal = row.get("cost_usd");
    let state_machine_id = row.get("state_machine__id");

    Ok(Step {
        id: row.get("id"),
        run_id: row.get("run_id"),
        name: row.get("name"),
        kind: parse_step_kind(kind_str)?,
        position: row.get::<i32, _>("position") as u32,
        status: FsmState::new(parse_step_status(state_name)?, state_machine_id),
        attempt: row.get::<i32, _>("attempt") as u32,
        input: row.get("input"),
        output: row.get("output"),
        error: row.get("error"),
        duration_ms: row.get::<i64, _>("duration_ms") as u64,
        cost_usd,
        input_tokens: row.get::<Option<i64>, _>("input_tokens").map(|v| v as u64),
        output_tokens: row.get::<Option<i64>, _>("output_tokens").map(|v| v as u64),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        started_at: row.get("started_at"),
        completed_at: row.get("completed_at"),
        debug_messages: row.get("debug_messages"),
        is_error_handler: row.try_get("is_error_handler").unwrap_or(false),
    })
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;

    fn user_actor() -> RunActor {
        RunActor::User {
            user_id: Uuid::now_v7(),
        }
    }

    fn api_key_actor() -> RunActor {
        RunActor::ApiKey {
            api_key_id: Uuid::now_v7(),
            user_id: Uuid::now_v7(),
        }
    }

    #[test]
    fn label_is_none_without_actor() {
        assert!(created_by_label(None, Some("alice".into()), None).is_none());
    }

    #[test]
    fn label_is_username_for_user_actor() {
        let actor = user_actor();
        assert_eq!(
            created_by_label(Some(&actor), Some("alice".into()), None),
            Some("alice".to_string())
        );
    }

    #[test]
    fn label_is_none_for_deleted_user() {
        let actor = user_actor();
        assert!(created_by_label(Some(&actor), None, None).is_none());
    }

    #[test]
    fn label_combines_key_and_owner() {
        let actor = api_key_actor();
        assert_eq!(
            created_by_label(Some(&actor), Some("alice".into()), Some("ci-deploy".into())),
            Some("ci-deploy (alice)".to_string())
        );
    }

    #[test]
    fn label_is_key_name_when_owner_deleted() {
        let actor = api_key_actor();
        assert_eq!(
            created_by_label(Some(&actor), None, Some("ci-deploy".into())),
            Some("ci-deploy".to_string())
        );
    }

    #[test]
    fn label_is_none_when_key_deleted() {
        let actor = api_key_actor();
        assert!(created_by_label(Some(&actor), Some("alice".into()), None).is_none());
    }
}
