//! Run-related DTOs and query parameters.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use ironflow_store::models::{Run, RunStatus, TriggerKind};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
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
    /// Cumulative cost cap for this run, in USD. `None` means no cap.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<f64>))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_cost_usd: Option<Decimal>,
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
            max_cost_usd: run.max_cost_usd,
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
    /// Page number (1-based).
    pub page: Option<u32>,
    /// Items per page.
    pub per_page: Option<u32>,
}

impl ListRunsQuery {
    /// Parse the comma-separated `label` param into a `HashMap`.
    pub fn parse_labels(&self) -> Option<HashMap<String, String>> {
        self.label.as_ref().and_then(|raw| {
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
}
