//! Step-related DTOs.

use chrono::{DateTime, Utc};
use ironflow_store::models::{Step, StepKind, StepStatus};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use super::ArtifactResponse;

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
    /// Which run attempt produced this step (1-based).
    ///
    /// A run retried twice exposes steps with `attempt` 1, 2 and 3. Steps from
    /// earlier attempts are kept so a failed attempt stays inspectable.
    pub attempt: u32,
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
    /// Verbose conversation trace for agent steps (thinking blocks, tool
    /// calls, tool results, per-turn usage). `None` when verbose mode was
    /// off or the step is not an agent step.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<serde_json::Value>))]
    pub debug_messages: Option<Value>,
    /// Files this step produced, downloadable through the artifact route.
    ///
    /// Empty when the step produced none or when artifact storage is not
    /// configured on the server.
    #[serde(default)]
    pub artifacts: Vec<ArtifactResponse>,
}

impl StepResponse {
    /// Build a response from a step entity with pre-resolved dependencies.
    ///
    /// Artifacts are left empty; use
    /// [`with_dependencies_and_artifacts`](Self::with_dependencies_and_artifacts)
    /// when they have been fetched.
    pub fn with_dependencies(step: Step, dependencies: Vec<Uuid>) -> Self {
        Self::with_dependencies_and_artifacts(step, dependencies, Vec::new())
    }

    /// Build a response from a step entity with its dependencies and artifacts.
    pub fn with_dependencies_and_artifacts(
        step: Step,
        dependencies: Vec<Uuid>,
        artifacts: Vec<ArtifactResponse>,
    ) -> Self {
        StepResponse {
            id: step.id,
            run_id: step.run_id,
            name: step.name,
            kind: step.kind,
            position: step.position,
            status: step.status.state,
            attempt: step.attempt,
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
            debug_messages: step.debug_messages,
            artifacts,
        }
    }
}

impl From<Step> for StepResponse {
    fn from(step: Step) -> Self {
        Self::with_dependencies(step, Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::models::{NewRun, NewStep, TriggerKind};
    use ironflow_store::store::RunStore;
    use serde_json::json;

    use super::*;

    /// A persisted step -- [`Step`] is `#[non_exhaustive]`, so it can only be
    /// obtained from a store.
    async fn step() -> Step {
        let store = InMemoryStore::new();
        let run = store
            .create_run(NewRun {
                created_by: None,
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 0,
                handler_version: None,
                labels: HashMap::new(),
                scheduled_at: None,
                idempotency_key: None,
                max_cost_usd: None,
            })
            .await
            .expect("create run")
            .into_run();

        store
            .create_step(NewStep {
                run_id: run.id,
                name: "build".to_string(),
                kind: StepKind::Shell,
                position: 0,
                input: None,
            })
            .await
            .expect("create step")
    }

    #[tokio::test]
    async fn a_step_without_artifacts_exposes_an_empty_list() {
        let response = StepResponse::from(step().await);
        assert!(response.artifacts.is_empty());
    }

    #[tokio::test]
    async fn artifacts_are_carried_through() {
        let step = step().await;
        let artifact = ArtifactResponse {
            id: Uuid::now_v7(),
            step_id: step.id,
            name: "report.html".to_string(),
            content_type: "text/html".to_string(),
            size_bytes: 1,
            sha256: "0".repeat(64),
            created_at: Utc::now(),
        };

        let response =
            StepResponse::with_dependencies_and_artifacts(step, Vec::new(), vec![artifact]);

        assert_eq!(response.artifacts.len(), 1);
        assert_eq!(response.artifacts[0].name, "report.html");
    }

    #[tokio::test]
    async fn artifacts_serialize_as_a_json_array() {
        let body = serde_json::to_value(StepResponse::from(step().await)).expect("serialize");
        assert!(body["artifacts"].is_array());
    }
}
