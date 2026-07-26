//! `POST /api/v1/runs` — Trigger a workflow.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use chrono::Utc;
use ironflow_auth::extractor::Authenticated;
use ironflow_engine::engine::EnqueueOptions;
use ironflow_engine::error::EngineError;
use ironflow_engine::notify::Event;
use ironflow_store::models::TriggerKind;
use serde_json::json;

use crate::entities::{CreateRunRequest, RunResponse};
use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Trigger a workflow by name.
///
/// Returns 201 Created with the newly enqueued run.
/// Returns 400 Bad Request if the workflow is unknown or the body is invalid.
/// Returns 429 Too Many Requests if the global monthly cost quota is exhausted.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        post,
        path = "/api/v1/runs",
        tags = ["runs"],
        request_body(content = CreateRunRequest, description = "Workflow to trigger"),
        responses(
            (status = 201, description = "Run created successfully", body = RunResponse),
            (status = 400, description = "Unknown workflow or invalid body"),
            (status = 401, description = "Unauthorized"),
            (status = 403, description = "Forbidden"),
            (status = 429, description = "Monthly cost quota exhausted")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn create_run(
    auth: Authenticated,
    State(state): State<AppState>,
    Json(req): Json<CreateRunRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if !auth.is_admin() {
        return Err(ApiError::Forbidden);
    }

    if !state
        .engine
        .handler_names()
        .contains(&req.workflow.as_str())
    {
        return Err(ApiError::BadRequest(format!(
            "unknown workflow: {}",
            req.workflow
        )));
    }

    req.validate().map_err(ApiError::BadRequest)?;

    let payload = req.payload.unwrap_or_else(|| json!({}));
    let labels = req.labels.unwrap_or_default();

    let run = state
        .engine
        .enqueue_handler_with_options(
            &req.workflow,
            TriggerKind::Api,
            payload,
            EnqueueOptions {
                max_retries: 3,
                labels,
                scheduled_at: req.scheduled_at,
                max_cost_usd: req.max_cost_usd,
            },
        )
        .await
        .map_err(|e| match e {
            EngineError::MonthlyBudgetExceeded { .. } => {
                ApiError::MonthlyBudgetExceeded(e.to_string())
            }
            other => ApiError::Internal(other.to_string()),
        })?;

    state.engine.event_publisher().publish(Event::RunCreated {
        run_id: run.id,
        workflow_name: run.workflow_name.clone(),
        at: Utc::now(),
    });

    let response = RunResponse::from(run);
    Ok((StatusCode::CREATED, ok(response)))
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, Response, StatusCode};
    use axum::routing::post;
    use http_body_util::BodyExt;
    use ironflow_auth::jwt::AccessToken;
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::budget::BudgetConfig;
    use ironflow_engine::context::WorkflowContext;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
    use ironflow_engine::notify::Event;
    use ironflow_store::memory::InMemoryStore;
    use rust_decimal::Decimal;
    use serde_json::{Value as JsonValue, json};
    use std::sync::Arc;
    use tokio::sync::broadcast;
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::*;

    fn make_auth_header(state: &AppState) -> String {
        let user_id = Uuid::now_v7();
        let token = AccessToken::for_user(user_id, "testuser", true, &state.jwt_config).unwrap();
        format!("Bearer {}", token.0)
    }

    struct TestWorkflow;

    impl WorkflowHandler for TestWorkflow {
        fn name(&self) -> &str {
            "test-workflow"
        }

        fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
            Box::pin(async move { Ok(()) })
        }
    }

    fn test_state() -> AppState {
        state_with_budget(BudgetConfig::new())
    }

    fn state_with_budget(budget: BudgetConfig) -> AppState {
        let store = Arc::new(InMemoryStore::new());
        let provider = Arc::new(ClaudeCodeProvider::new());
        let mut engine = Engine::new(store.clone(), provider).with_budget_config(budget);
        engine.register(TestWorkflow).unwrap();
        let jwt_config = Arc::new(ironflow_auth::jwt::JwtConfig {
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

    #[tokio::test]
    async fn create_run_success() {
        let state = test_state();
        let auth_header = make_auth_header(&state);
        let app = Router::new().route("/", post(create_run)).with_state(state);

        let req = Request::builder()
            .uri("/")
            .method("POST")
            .header("content-type", "application/json")
            .header("authorization", auth_header)
            .body(Body::from(
                serde_json::to_string(&json!({
                    "workflow": "test-workflow",
                    "payload": {"key": "value"}
                }))
                .unwrap(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = serde_json::from_slice(&body).unwrap();
        assert_eq!(json_val["data"]["workflow_name"], "test-workflow");
    }

    #[tokio::test]
    async fn create_run_unknown_workflow() {
        let state = test_state();
        let auth_header = make_auth_header(&state);
        let app = Router::new().route("/", post(create_run)).with_state(state);

        let req = Request::builder()
            .uri("/")
            .method("POST")
            .header("content-type", "application/json")
            .header("authorization", auth_header)
            .body(Body::from(
                serde_json::to_string(&json!({
                    "workflow": "unknown-workflow",
                    "payload": {}
                }))
                .unwrap(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    /// Send a `POST /` with the given JSON body against a router built on `state`.
    async fn post_run(state: AppState, body: JsonValue) -> Response<Body> {
        let auth_header = make_auth_header(&state);
        let app = Router::new().route("/", post(create_run)).with_state(state);

        let req = Request::builder()
            .uri("/")
            .method("POST")
            .header("content-type", "application/json")
            .header("authorization", auth_header)
            .body(Body::from(serde_json::to_string(&body).unwrap()))
            .unwrap();

        app.oneshot(req).await.unwrap()
    }

    #[tokio::test]
    async fn create_run_persists_and_returns_max_cost_usd() {
        let resp = post_run(
            test_state(),
            json!({"workflow": "test-workflow", "max_cost_usd": 2.5}),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::CREATED);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = serde_json::from_slice(&body).unwrap();
        assert_eq!(json_val["data"]["max_cost_usd"], 2.5);
    }

    #[tokio::test]
    async fn create_run_without_max_cost_omits_the_field() {
        let resp = post_run(test_state(), json!({"workflow": "test-workflow"})).await;
        assert_eq!(resp.status(), StatusCode::CREATED);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = serde_json::from_slice(&body).unwrap();
        assert!(json_val["data"].get("max_cost_usd").is_none());
    }

    #[tokio::test]
    async fn create_run_applies_server_default_max_cost() {
        let state =
            state_with_budget(BudgetConfig::new().default_run_max_cost_usd(Decimal::new(125, 2)));
        let resp = post_run(state, json!({"workflow": "test-workflow"})).await;
        assert_eq!(resp.status(), StatusCode::CREATED);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = serde_json::from_slice(&body).unwrap();
        assert_eq!(json_val["data"]["max_cost_usd"], 1.25);
    }

    #[tokio::test]
    async fn create_run_rejects_negative_max_cost() {
        let resp = post_run(
            test_state(),
            json!({"workflow": "test-workflow", "max_cost_usd": -1.0}),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = serde_json::from_slice(&body).unwrap();
        assert_eq!(json_val["error"]["code"], "BAD_REQUEST");
        assert!(
            json_val["error"]["message"]
                .as_str()
                .unwrap()
                .contains("max_cost_usd")
        );
    }

    #[tokio::test]
    async fn create_run_accepts_zero_max_cost() {
        let resp = post_run(
            test_state(),
            json!({"workflow": "test-workflow", "max_cost_usd": 0}),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    #[tokio::test]
    async fn create_run_returns_429_when_monthly_quota_exhausted() {
        // Quota of $0: any accumulated cost (including zero) meets the limit.
        let state = state_with_budget(BudgetConfig::new().monthly_cost_limit_usd(Decimal::ZERO));
        let resp = post_run(state, json!({"workflow": "test-workflow"})).await;
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = serde_json::from_slice(&body).unwrap();
        assert_eq!(json_val["error"]["code"], "MONTHLY_BUDGET_EXCEEDED");
    }

    #[tokio::test]
    async fn create_run_succeeds_when_monthly_quota_has_room() {
        let state =
            state_with_budget(BudgetConfig::new().monthly_cost_limit_usd(Decimal::new(10000, 2)));
        let resp = post_run(state, json!({"workflow": "test-workflow"})).await;
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    #[tokio::test]
    async fn create_run_without_payload() {
        let state = test_state();
        let auth_header = make_auth_header(&state);
        let app = Router::new().route("/", post(create_run)).with_state(state);

        let req = Request::builder()
            .uri("/")
            .method("POST")
            .header("content-type", "application/json")
            .header("authorization", auth_header)
            .body(Body::from(
                serde_json::to_string(&json!({
                    "workflow": "test-workflow"
                }))
                .unwrap(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
    }
}
