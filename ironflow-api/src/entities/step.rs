//! Step-related DTOs.

use chrono::{DateTime, Utc};
use ironflow_store::models::{
    ApprovalRequirement, Assignee, Step, StepApproval, StepKind, StepStatus,
};
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
    /// Deterministic trace ID for log correlation.
    pub trace_id: Uuid,
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
    /// Uncached input token count (agent steps).
    pub input_tokens: Option<u64>,
    /// Input tokens served from the prompt cache (agent steps).
    pub cache_read_input_tokens: Option<u64>,
    /// Input tokens written to the prompt cache (agent steps).
    pub cache_creation_input_tokens: Option<u64>,
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
    /// When this approval gate expires, if it carries an SLA deadline.
    pub approval_deadline_at: Option<DateTime<Utc>>,
    /// Seconds left before the gate escalates. Clamped at 0, `None` when the
    /// step has no deadline.
    pub approval_seconds_remaining: Option<i64>,
    /// Who the approval is currently assigned to.
    ///
    /// Serialized as a prefixed string: `user:{name}` or `group:{name}`.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub approval_assignee: Option<Assignee>,
    /// Approvers the workflow handler required when the gate opened. `None`
    /// for a gate opened without approvers.
    #[serde(default)]
    pub approval_requirement: Option<ApprovalRequirement>,
    /// Votes cast on the approval gate so far, at most one per user.
    #[serde(default)]
    pub approvals: Vec<StepApproval>,
    /// Distinct approvals the gate needs: the requirement's count, `1` for an
    /// approval step without rules, `None` for any other step kind.
    #[serde(default)]
    pub approvals_required: Option<u32>,
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
        let approval_seconds_remaining = step
            .approval_deadline_at
            .map(|at| (at - Utc::now()).num_seconds().max(0));
        let approvals_required = match (&step.kind, &step.approval_requirement) {
            (_, Some(requirement)) => Some(requirement.required_approvers),
            (StepKind::Approval, None) => Some(1),
            _ => None,
        };

        StepResponse {
            id: step.id,
            trace_id: step.trace_id,
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
            cache_read_input_tokens: step.cache_read_input_tokens,
            cache_creation_input_tokens: step.cache_creation_input_tokens,
            output_tokens: step.output_tokens,
            created_at: step.created_at,
            updated_at: step.updated_at,
            started_at: step.started_at,
            completed_at: step.completed_at,
            dependencies,
            debug_messages: step.debug_messages,
            artifacts,
            approval_deadline_at: step.approval_deadline_at,
            approval_seconds_remaining,
            approval_assignee: step.approval_assignee,
            approval_requirement: step.approval_requirement,
            approvals: step.approvals,
            approvals_required,
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

    use chrono::TimeDelta;
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::models::{NewRun, NewStep, StepUpdate, TriggerKind, step_trace_id};
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
                trace_id: step_trace_id(run.id, "build", 0),
                name: "build".to_string(),
                kind: StepKind::Shell,
                position: 0,
                input: None,
                is_error_handler: false,
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

    /// An approval step whose deadline is `offset_secs` from now.
    async fn gate_with_deadline(offset_secs: i64) -> Step {
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

        let step = store
            .create_step(NewStep {
                run_id: run.id,
                trace_id: step_trace_id(run.id, "prod-gate", 0),
                name: "prod-gate".to_string(),
                kind: StepKind::Approval,
                position: 0,
                input: None,
                is_error_handler: false,
            })
            .await
            .expect("create step");

        store
            .update_step(
                step.id,
                StepUpdate {
                    status: Some(StepStatus::Running),
                    ..StepUpdate::default()
                },
            )
            .await
            .expect("to running");
        store
            .update_step(
                step.id,
                StepUpdate {
                    status: Some(StepStatus::AwaitingApproval),
                    approval_deadline_at: Some(Utc::now() + TimeDelta::seconds(offset_secs)),
                    approval_assignee: Some(Assignee::group("release-managers")),
                    ..StepUpdate::default()
                },
            )
            .await
            .expect("arm timer");

        store.get_step(step.id).await.expect("get").expect("exists")
    }

    #[tokio::test]
    async fn a_step_without_a_deadline_reports_no_sla() {
        let response = StepResponse::from(step().await);
        assert!(response.approval_deadline_at.is_none());
        assert!(response.approval_seconds_remaining.is_none());
        assert!(response.approval_assignee.is_none());
        assert!(response.approval_requirement.is_none());
        assert!(response.approvals.is_empty());
    }

    #[tokio::test]
    async fn a_shell_step_requires_no_approvals() {
        let response = StepResponse::from(step().await);
        assert_eq!(response.approvals_required, None);
    }

    #[tokio::test]
    async fn a_rule_less_approval_step_requires_one_approval() {
        let response = StepResponse::from(gate_with_deadline(3600).await);
        assert!(response.approval_requirement.is_none());
        assert_eq!(response.approvals_required, Some(1));
    }

    #[tokio::test]
    async fn an_approval_requirement_sets_the_required_count() {
        let mut gate = gate_with_deadline(3600).await;
        let requirement = ApprovalRequirement {
            reason: Some("amount > 100k".to_string()),
            required_approvers: 3,
            approver_groups: vec!["finance".to_string()],
        };
        gate.approval_requirement = Some(requirement.clone());
        gate.approvals = vec![StepApproval {
            user_id: Uuid::now_v7(),
            approved_by: "alice".to_string(),
            at: Utc::now(),
        }];

        let response = StepResponse::from(gate);

        assert_eq!(response.approvals_required, Some(3));
        assert_eq!(response.approval_requirement, Some(requirement));
        assert_eq!(response.approvals.len(), 1);
        assert_eq!(response.approvals[0].approved_by, "alice");
    }

    #[tokio::test]
    async fn a_future_deadline_reports_the_remaining_seconds() {
        let response = StepResponse::from(gate_with_deadline(3600).await);

        assert!(response.approval_deadline_at.is_some());
        let remaining = response
            .approval_seconds_remaining
            .expect("a deadline yields a countdown");
        assert!(remaining > 0 && remaining <= 3600, "got {remaining}");
        assert_eq!(
            response.approval_assignee,
            Some(Assignee::group("release-managers"))
        );
    }

    #[tokio::test]
    async fn a_past_deadline_clamps_the_countdown_at_zero() {
        let response = StepResponse::from(gate_with_deadline(-3600).await);
        assert_eq!(response.approval_seconds_remaining, Some(0));
    }

    #[tokio::test]
    async fn trace_id_is_exposed_in_step_response() {
        let s = step().await;
        let expected_trace_id = s.trace_id;
        let response = StepResponse::from(s);

        assert_eq!(response.trace_id, expected_trace_id);

        let body = serde_json::to_value(&response).expect("serialize");
        assert!(body["trace_id"].is_string());
    }
}
