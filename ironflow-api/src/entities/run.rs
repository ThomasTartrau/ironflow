//! Run-related DTOs and query parameters.

use chrono::{DateTime, Utc};
use ironflow_store::models::{Run, RunStatus, TriggerKind};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::StepResponse;

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
}

impl From<Run> for RunResponse {
    fn from(run: Run) -> Self {
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
        }
    }
}

/// Run detail response — includes steps.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Serialize)]
pub struct RunDetailResponse {
    /// The run.
    pub run: RunResponse,
    /// Associated steps, ordered by position.
    pub steps: Vec<StepResponse>,
}

/// Query parameters for listing runs.
#[cfg_attr(feature = "openapi", derive(utoipa::IntoParams, utoipa::ToSchema))]
#[derive(Debug, Deserialize)]
pub struct ListRunsQuery {
    /// Filter by workflow name.
    pub workflow: Option<String>,
    /// Filter by run status.
    pub status: Option<RunStatus>,
    /// When `true`, only return runs with at least one step.
    /// When `false`, only return runs with no steps.
    pub has_steps: Option<bool>,
    /// Page number (1-based).
    pub page: Option<u32>,
    /// Items per page.
    pub per_page: Option<u32>,
}
