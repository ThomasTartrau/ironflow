//! `POST /api/v1/workflows/:name/plan` — Build a workflow execution plan.

use axum::Json;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use ironflow_auth::extractor::Authenticated;
use ironflow_engine::error::EngineError;
use ironflow_engine::plan::{
    ConditionResult, DEFAULT_ESTIMATE_SAMPLE_RUNS, DEFAULT_PLAN_MAX_DEPTH, ExecutionPlan,
    PlanOptions, PlannedStep,
};
use ironflow_store::entities::StepKind;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Highest sub-workflow expansion depth the API accepts.
const MAX_PLAN_DEPTH: u32 = 10;

/// Request body for building a workflow execution plan.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Default, Deserialize, Serialize)]
pub struct PlanWorkflowRequest {
    /// Input payload the plan is computed for. Defaults to `{}`.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<std::collections::HashMap<String, serde_json::Value>>))]
    #[serde(default)]
    pub payload: Option<Value>,
    /// How deep sub-workflows are expanded. Defaults to 3, capped at 10.
    #[serde(default)]
    pub max_depth: Option<u32>,
    /// Estimate step durations from run history. Defaults to `true`.
    #[serde(default)]
    pub estimate_durations: Option<bool>,
}

/// Outcome of a branch condition as recorded by the planner.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Serialize)]
pub struct ConditionResponse {
    /// Condition state: `evaluated`, `skipped` or `unevaluable`.
    pub state: String,
    /// Expression the handler declared, when the planner knows one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
    /// What the expression evaluated to, for an `evaluated` condition.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<bool>,
    /// Why the step is skipped, or why the condition cannot be evaluated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// One step the planner expects the run to create.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Serialize)]
pub struct PlannedStepResponse {
    /// Step name as the handler declares it.
    pub name: String,
    /// Step kind, or the name of a custom operation.
    pub kind: String,
    /// Workflow that owns this step.
    pub workflow: String,
    /// Sub-workflow nesting depth; `0` for the top-level workflow.
    pub depth: u32,
    /// Names of the steps this one runs after.
    pub depends_on: Vec<String>,
    /// Branch condition recorded for this step, when the handler declared one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<ConditionResponse>,
    /// Parallel wave this step belongs to, when it runs concurrently.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parallel_group: Option<String>,
    /// Average duration of this step in past completed runs, in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_duration_ms: Option<u64>,
}

/// The execution plan of one workflow for one input payload.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Serialize)]
pub struct ExecutionPlanResponse {
    /// Workflow the plan was built for.
    pub workflow: String,
    /// Steps the run is expected to create, in execution order.
    pub steps: Vec<PlannedStepResponse>,
    /// Sum of the step estimates, counting each parallel wave once.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_duration_ms: Option<u64>,
    /// Sub-workflow expansion depth used for this plan.
    pub max_depth: u32,
    /// `true` when the step cap or the depth limit cut the plan short.
    pub truncated: bool,
    /// Why the plan stopped early, when it did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub incomplete_reason: Option<String>,
}

/// Render a [`StepKind`] the way the rest of the API does: a plain string.
fn kind_label(kind: &StepKind) -> String {
    match kind {
        StepKind::Shell => "shell".to_string(),
        StepKind::Http => "http".to_string(),
        StepKind::Agent => "agent".to_string(),
        StepKind::Workflow => "workflow".to_string(),
        StepKind::Approval => "approval".to_string(),
        StepKind::Decision => "decision".to_string(),
        StepKind::Custom(name) => name.clone(),
    }
}

impl From<ConditionResult> for ConditionResponse {
    fn from(condition: ConditionResult) -> Self {
        match condition {
            ConditionResult::Evaluated { expression, value } => Self {
                state: "evaluated".to_string(),
                expression: Some(expression),
                value: Some(value),
                reason: None,
            },
            ConditionResult::Skipped { reason } => Self {
                state: "skipped".to_string(),
                expression: None,
                value: None,
                reason: Some(reason),
            },
            ConditionResult::Unevaluable { expression, reason } => Self {
                state: "unevaluable".to_string(),
                expression: Some(expression),
                value: None,
                reason: Some(reason),
            },
        }
    }
}

impl From<PlannedStep> for PlannedStepResponse {
    fn from(step: PlannedStep) -> Self {
        let estimated_duration_ms = step.estimated_duration_ms();
        Self {
            name: step.name,
            kind: kind_label(&step.kind),
            workflow: step.workflow,
            depth: step.depth,
            depends_on: step.depends_on,
            condition: step.condition.map(ConditionResponse::from),
            parallel_group: step.parallel_group,
            estimated_duration_ms,
        }
    }
}

impl From<ExecutionPlan> for ExecutionPlanResponse {
    fn from(plan: ExecutionPlan) -> Self {
        let estimated_duration_ms = plan.estimated_duration_ms();
        Self {
            workflow: plan.workflow,
            steps: plan
                .steps
                .into_iter()
                .map(PlannedStepResponse::from)
                .collect(),
            estimated_duration_ms,
            max_depth: plan.max_depth,
            truncated: plan.truncated,
            incomplete_reason: plan.incomplete_reason,
        }
    }
}

/// Build a workflow execution plan without running it.
///
/// # Errors
///
/// - 400 if `max_depth` is out of range or the payload is not a JSON object
/// - 404 if the workflow is not registered
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        post,
        path = "/api/v1/workflows/{name}/plan",
        tags = ["workflows"],
        params(("name" = String, Path, description = "Workflow name")),
        request_body(content = PlanWorkflowRequest, description = "Input payload and planning options"),
        responses(
            (status = 200, description = "Execution plan", body = ExecutionPlanResponse),
            (status = 400, description = "Invalid payload or max_depth"),
            (status = 401, description = "Unauthorized"),
            (status = 404, description = "Workflow not found")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn plan_workflow(
    _auth: Authenticated,
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(req): Json<PlanWorkflowRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if !state.engine.handler_names().contains(&name.as_str()) {
        return Err(ApiError::WorkflowNotFound(name));
    }

    let max_depth = req.max_depth.unwrap_or(DEFAULT_PLAN_MAX_DEPTH);
    if max_depth == 0 || max_depth > MAX_PLAN_DEPTH {
        return Err(ApiError::BadRequest(format!(
            "max_depth must be between 1 and {MAX_PLAN_DEPTH}"
        )));
    }

    let payload = req.payload.unwrap_or_else(|| json!({}));
    if !payload.is_object() {
        return Err(ApiError::BadRequest(
            "payload must be a JSON object".to_string(),
        ));
    }

    let options = PlanOptions {
        max_depth,
        estimate_durations: req.estimate_durations.unwrap_or(true),
        sample_runs: DEFAULT_ESTIMATE_SAMPLE_RUNS,
    };

    let plan = state
        .engine
        .plan_handler(&name, payload, options)
        .await
        .map_err(|e| match e {
            EngineError::InvalidWorkflow(msg) => ApiError::BadRequest(msg),
            EngineError::Store(err) => ApiError::from(err),
            other => ApiError::Internal(other.to_string()),
        })?;

    Ok(ok(ExecutionPlanResponse::from(plan)))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::response::Response;
    use axum::routing::post;
    use http_body_util::BodyExt;
    use ironflow_auth::jwt::{AccessToken, JwtConfig};
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::config::{ShellConfig, StepConfig};
    use ironflow_engine::context::WorkflowContext;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
    use ironflow_engine::notify::Event;
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::models::RunFilter;
    use serde_json::{Value as JsonValue, from_slice};
    use tokio::sync::broadcast;
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::*;

    struct PlannedWorkflow;

    impl WorkflowHandler for PlannedWorkflow {
        fn name(&self) -> &str {
            "planned"
        }

        fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
            Box::pin(async move {
                ctx.shell("build", ShellConfig::new("echo build")).await?;
                ctx.parallel(
                    vec![
                        ("test", StepConfig::Shell(ShellConfig::new("echo test"))),
                        ("lint", StepConfig::Shell(ShellConfig::new("echo lint"))),
                    ],
                    true,
                )
                .await?;
                Ok(())
            })
        }
    }

    #[derive(Deserialize)]
    struct DeployInput {
        env: String,
    }

    struct ConditionalWorkflow;

    impl WorkflowHandler for ConditionalWorkflow {
        fn name(&self) -> &str {
            "conditional"
        }

        fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
            Box::pin(async move {
                if ctx
                    .when("production run", |i: &DeployInput| i.env == "prod")
                    .await?
                {
                    ctx.shell("deploy", ShellConfig::new("echo deploy")).await?;
                } else {
                    ctx.skip("deploy", "not prod").await?;
                }
                Ok(())
            })
        }
    }

    fn test_state() -> AppState {
        let store = Arc::new(InMemoryStore::new());
        let provider = Arc::new(ClaudeCodeProvider::new());
        let mut engine = Engine::new(store.clone(), provider);
        engine.register(PlannedWorkflow).unwrap();
        engine.register(ConditionalWorkflow).unwrap();
        let jwt_config = Arc::new(JwtConfig {
            secret: "test-secret".to_string(),
            access_token_ttl_secs: 900,
            refresh_token_ttl_secs: 604800,
            cookie_domain: None,
            cookie_secure: false,
        });
        let (event_sender, _) = broadcast::channel::<Event>(1);
        AppState::new(
            store,
            Arc::new(engine),
            jwt_config,
            "test-worker-token".to_string(),
            event_sender,
        )
    }

    fn make_auth_header(state: &AppState) -> String {
        let user_id = Uuid::now_v7();
        let token = AccessToken::for_user(user_id, "testuser", false, &state.jwt_config).unwrap();
        format!("Bearer {}", token.0)
    }

    fn app(state: AppState) -> Router {
        Router::new()
            .route("/api/v1/workflows/{name}/plan", post(plan_workflow))
            .with_state(state)
    }

    fn plan_request(name: &str, auth: Option<&str>, body: JsonValue) -> Request<Body> {
        let mut builder = Request::builder()
            .method("POST")
            .uri(format!("/api/v1/workflows/{name}/plan"))
            .header("content-type", "application/json");
        if let Some(header) = auth {
            builder = builder.header("authorization", header);
        }
        builder.body(Body::from(body.to_string())).unwrap()
    }

    async fn body_json(response: Response) -> JsonValue {
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn plan_returns_steps_for_registered_workflow() {
        let state = test_state();
        let auth = make_auth_header(&state);
        let response = app(state)
            .oneshot(plan_request("planned", Some(&auth), json!({})))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = body_json(response).await;
        assert_eq!(body["data"]["workflow"], "planned");
        let steps = body["data"]["steps"].as_array().unwrap();
        assert_eq!(steps.len(), 3);
        assert_eq!(steps[0]["name"], "build");
        assert_eq!(steps[0]["kind"], "shell");
        assert_eq!(steps[1]["name"], "test");
        assert_eq!(steps[2]["name"], "lint");
        assert_eq!(body["data"]["truncated"], false);
    }

    #[tokio::test]
    async fn plan_marks_parallel_group() {
        let state = test_state();
        let auth = make_auth_header(&state);
        let response = app(state)
            .oneshot(plan_request("planned", Some(&auth), json!({})))
            .await
            .unwrap();

        let body = body_json(response).await;
        let steps = body["data"]["steps"].as_array().unwrap();
        assert!(steps[0].get("parallel_group").is_none());
        assert_eq!(steps[1]["parallel_group"], "parallel-1");
        assert_eq!(steps[2]["parallel_group"], "parallel-1");
    }

    #[tokio::test]
    async fn plan_evaluates_condition_from_payload() {
        let state = test_state();
        let auth = make_auth_header(&state);
        let router = app(state);

        let response = router
            .clone()
            .oneshot(plan_request(
                "conditional",
                Some(&auth),
                json!({"payload": {"env": "prod"}}),
            ))
            .await
            .unwrap();
        let body = body_json(response).await;
        let step = &body["data"]["steps"][0];
        assert_eq!(step["kind"], "shell");
        assert_eq!(step["condition"]["state"], "evaluated");
        assert_eq!(step["condition"]["expression"], "production run");
        assert_eq!(step["condition"]["value"], true);

        let response = router
            .oneshot(plan_request(
                "conditional",
                Some(&auth),
                json!({"payload": {"env": "dev"}}),
            ))
            .await
            .unwrap();
        let body = body_json(response).await;
        let step = &body["data"]["steps"][0];
        assert_eq!(step["kind"], "skip");
        assert_eq!(step["condition"]["state"], "skipped");
        assert_eq!(step["condition"]["reason"], "not prod");
    }

    #[tokio::test]
    async fn plan_unknown_workflow_returns_404() {
        let state = test_state();
        let auth = make_auth_header(&state);
        let response = app(state)
            .oneshot(plan_request("nonexistent", Some(&auth), json!({})))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn plan_rejects_zero_and_excessive_max_depth() {
        let state = test_state();
        let auth = make_auth_header(&state);
        let router = app(state);

        let response = router
            .clone()
            .oneshot(plan_request(
                "planned",
                Some(&auth),
                json!({"max_depth": 0}),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let response = router
            .oneshot(plan_request(
                "planned",
                Some(&auth),
                json!({"max_depth": 11}),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn plan_rejects_non_object_payload() {
        let state = test_state();
        let auth = make_auth_header(&state);
        let response = app(state)
            .oneshot(plan_request(
                "planned",
                Some(&auth),
                json!({"payload": [1, 2, 3]}),
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn plan_requires_authentication() {
        let state = test_state();
        let response = app(state)
            .oneshot(plan_request("planned", None, json!({})))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn plan_does_not_create_a_run() {
        let state = test_state();
        let store = state.store.clone();
        let auth = make_auth_header(&state);
        let response = app(state)
            .oneshot(plan_request("planned", Some(&auth), json!({})))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let runs = store.list_runs(RunFilter::default(), 1, 10).await.unwrap();
        assert!(runs.items.is_empty());
    }
}
