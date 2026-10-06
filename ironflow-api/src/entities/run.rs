//! Run-related DTOs and query parameters.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use ironflow_store::models::{ConcurrencyLimit, Run, RunStatus, TriggerKind};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use super::{CreatedBy, StepResponse};

/// Run response DTO — public API representation of a run.
///
/// Maps from the internal [`Run`] model, exposing only necessary fields.
///
/// # Examples
///
/// ```
/// use ironflow_store::models::{Run, RunStatus, TriggerKind};
/// use ironflow_api::entities::RunResponse;
/// ```
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Serialize, Deserialize)]
pub struct RunResponse {
    /// Unique run identifier.
    pub id: Uuid,
    /// Workflow name.
    pub workflow_name: String,
    /// Current status.
    pub status: RunStatus,
    /// How the run was triggered.
    pub trigger: TriggerKind,
    /// Optional error message.
    pub error: Option<String>,
    /// Number of times retried.
    pub retry_count: u32,
    /// Maximum allowed retries.
    pub max_retries: u32,
    /// Aggregated cost in USD.
    #[cfg_attr(feature = "openapi", schema(value_type = f64))]
    pub cost_usd: Decimal,
    /// Total duration in milliseconds.
    pub duration_ms: u64,
    /// When created.
    pub created_at: DateTime<Utc>,
    /// When last updated.
    pub updated_at: DateTime<Utc>,
    /// When execution started.
    pub started_at: Option<DateTime<Utc>>,
    /// When execution completed.
    pub completed_at: Option<DateTime<Utc>>,
    /// Version of the handler that created this run.
    pub handler_version: Option<String>,
    /// User-defined key-value labels.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub labels: HashMap<String, String>,
    /// Scheduled execution time. `None` means the run executed immediately.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduled_at: Option<DateTime<Utc>>,
    /// Who triggered the run. Always present.
    pub created_by: CreatedBy,
    /// Idempotency key that produced this run, when one was supplied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    /// Exclusivity key held by this run until it reaches a terminal state,
    /// when one was supplied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub concurrency_key: Option<String>,
    /// Concurrency groups the run belongs to, with the limit it was created
    /// with. Omitted when the run belongs to no group.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub concurrency_limits: Vec<ConcurrencyLimit>,
    /// Cumulative cost cap for this run, in USD. `None` means no cap.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<f64>))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_cost_usd: Option<Decimal>,
    /// Typed output the handler set with `WorkflowContext::set_output`.
    ///
    /// Written when the run ends (completed, warning, failed or cancelled).
    /// Omitted when the handler set no output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<Value>,
}

impl From<Run> for RunResponse {
    fn from(run: Run) -> Self {
        let created_by = CreatedBy::from(&run);
        RunResponse {
            id: run.id,
            workflow_name: run.workflow_name,
            status: run.status.state,
            trigger: run.trigger,
            error: run.error,
            retry_count: run.retry_count,
            max_retries: run.max_retries,
            cost_usd: run.cost_usd,
            duration_ms: run.duration_ms,
            created_at: run.created_at,
            updated_at: run.updated_at,
            started_at: run.started_at,
            completed_at: run.completed_at,
            handler_version: run.handler_version,
            labels: run.labels,
            scheduled_at: run.scheduled_at,
            created_by,
            idempotency_key: run.idempotency_key,
            concurrency_key: run.concurrency_key,
            concurrency_limits: run.concurrency_limits,
            max_cost_usd: run.max_cost_usd,
            output: run.output,
        }
    }
}

/// Run detail response — includes steps and payload.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Serialize)]
pub struct RunDetailResponse {
    /// The run.
    pub run: RunResponse,
    /// Associated steps, ordered by position.
    pub steps: Vec<StepResponse>,
    /// Input payload that triggered this run.
    pub payload: serde_json::Value,
    /// Sub-workflow runs below this run, at any depth, that are not finished:
    /// cancelling the run cancels them too.
    pub active_descendant_count: u64,
}

/// Response of `POST /api/v1/runs/:id/cancel`: the cancelled run, with the
/// sub-workflow runs cancelled along with it.
///
/// The run's fields stay at the top level, as before the descendants were
/// listed, so existing clients read the same document.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Serialize)]
pub struct CancelRunResponse {
    /// The cancelled run.
    #[serde(flatten)]
    pub run: RunResponse,
    /// Sub-workflow runs below it that this request cancelled, oldest first.
    /// Empty when none was still active.
    pub cancelled_descendants: Vec<Uuid>,
}

/// Query parameters for listing runs.
#[cfg_attr(feature = "openapi", derive(utoipa::IntoParams, utoipa::ToSchema))]
#[derive(Debug, Deserialize)]
pub struct ListRunsQuery {
    /// Filter by workflow name.
    pub workflow: Option<String>,
    /// Filter by run status.
    pub status: Option<RunStatus>,
    /// Filter by step presence (only applies to completed/cancelled runs).
    /// Non-terminal runs (pending, running, etc.) are always included.
    /// When `true`, only return completed/cancelled runs that have steps.
    /// When `false`, only return completed/cancelled runs without steps.
    pub has_steps: Option<bool>,
    /// Filter by labels. Comma-separated `key:value` pairs.
    pub label: Option<String>,
    /// Filter by author: the user ID that triggered the run.
    ///
    /// Also matches runs triggered by one of that user's API keys.
    pub created_by: Option<Uuid>,
    /// Filter by concurrency group: only runs that belong to this group.
    pub concurrency_group: Option<String>,
    /// Page number (1-based).
    pub page: Option<u32>,
    /// Items per page.
    pub per_page: Option<u32>,
}

impl ListRunsQuery {
    /// Parse the comma-separated `label` param into a `HashMap`.
    pub fn parse_labels(&self) -> Option<HashMap<String, String>> {
        parse_label_param(&self.label)
    }
}

/// Parse a comma-separated `key:value` label query param into a `HashMap`.
///
/// Entries without a `:` are ignored. Returns `None` when the param is
/// absent or contains no valid entry.
pub(crate) fn parse_label_param(raw: &Option<String>) -> Option<HashMap<String, String>> {
    raw.as_ref().and_then(|raw| {
        let mut map = HashMap::new();
        for entry in raw.split(',') {
            let entry = entry.trim();
            if let Some((k, v)) = entry.split_once(':') {
                map.insert(k.to_string(), v.to_string());
            }
        }
        if map.is_empty() { None } else { Some(map) }
    })
}
