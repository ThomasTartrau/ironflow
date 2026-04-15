//! Step-related DTOs.

use chrono::{DateTime, Utc};
use ironflow_store::models::{Step, StepKind, StepStatus};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// Step response DTO — public API representation of a step.
///
/// # Examples
///
/// ```
/// use ironflow_store::models::Step;
/// use ironflow_api::entities::StepResponse;
/// ```
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Serialize, Deserialize)]
pub struct StepResponse {
    /// Unique step identifier.
    pub id: Uuid,
    /// Parent run ID.
    pub run_id: Uuid,
    /// Step name.
    pub name: String,
    /// Step operation type.
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub kind: StepKind,
    /// Execution order (0-based).
    pub position: u32,
    /// Current status.
    pub status: StepStatus,
    /// Input configuration.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<std::collections::HashMap<String, serde_json::Value>>))]
    pub input: Option<Value>,
    /// Step output.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<std::collections::HashMap<String, serde_json::Value>>))]
    pub output: Option<Value>,
    /// Optional error message.
    pub error: Option<String>,
    /// Execution duration in milliseconds.
    pub duration_ms: u64,
    /// Cost in USD.
    #[cfg_attr(feature = "openapi", schema(value_type = f64))]
    pub cost_usd: Decimal,
    /// Input token count (agent steps).
    pub input_tokens: Option<u64>,
    /// Output token count (agent steps).
    pub output_tokens: Option<u64>,
    /// When created.
    pub created_at: DateTime<Utc>,
    /// When updated.
    pub updated_at: DateTime<Utc>,
    /// When execution started.
    pub started_at: Option<DateTime<Utc>>,
    /// When execution completed.
    pub completed_at: Option<DateTime<Utc>>,
    /// IDs of steps this step depends on (direct dependencies).
    pub dependencies: Vec<Uuid>,
}

impl StepResponse {
    /// Build a response from a step entity with pre-resolved dependencies.
    pub fn with_dependencies(step: Step, dependencies: Vec<Uuid>) -> Self {
        StepResponse {
            id: step.id,
            run_id: step.run_id,
            name: step.name,
            kind: step.kind,
            position: step.position,
            status: step.status.state,
            input: step.input,
            output: step.output,
            error: step.error,
            duration_ms: step.duration_ms,
            cost_usd: step.cost_usd,
            input_tokens: step.input_tokens,
            output_tokens: step.output_tokens,
            created_at: step.created_at,
            updated_at: step.updated_at,
            started_at: step.started_at,
            completed_at: step.completed_at,
            dependencies,
        }
    }
}

impl From<Step> for StepResponse {
    fn from(step: Step) -> Self {
        Self::with_dependencies(step, Vec::new())
    }
}
