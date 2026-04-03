//! `POST /api/v1/runs` — Trigger a workflow.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use ironflow_auth::extractor::AuthenticatedUser;
use ironflow_store::models::TriggerKind;
use serde_json::json;

use crate::entities::{CreateRunRequest, RunResponse};
use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Trigger a workflow by name.
///
/// Returns 201 Created with the newly enqueued run.
/// Returns 400 Bad Request if the workflow is unknown.
pub async fn create_run(
    _user: AuthenticatedUser,
    State(state): State<AppState>,
    Json(req): Json<CreateRunRequest>,
) -> Result<impl IntoResponse, ApiError> {
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

    let payload = req.payload.unwrap_or_else(|| json!({}));

    let run = state
        .engine
        .enqueue_handler(&req.workflow, TriggerKind::Api, payload, 3)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let response = RunResponse::from(run);
    Ok((StatusCode::CREATED, ok(response)))
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::post;
    use http_body_util::BodyExt;
    use ironflow_auth::jwt::AccessToken;
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::context::WorkflowContext;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
    use ironflow_store::memory::InMemoryStore;
    use serde_json::{Value as JsonValue, json};
    use std::sync::Arc;
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::*;

    fn make_auth_header(state: &AppState) -> String {
        let user_id = Uuid::now_v7();
        let token = AccessToken::for_user(user_id, "testuser", false, &state.jwt_config).unwrap();
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
        let store = Arc::new(InMemoryStore::new());
        let user_store: Arc<dyn ironflow_store::user_store::UserStore> =
            Arc::new(InMemoryStore::new());
        let provider = Arc::new(ClaudeCodeProvider::new());
        let mut engine = Engine::new(store.clone(), provider);
        engine.register(TestWorkflow).unwrap();
        let jwt_config = Arc::new(ironflow_auth::jwt::JwtConfig {
            secret: "test-secret".to_string(),
            access_token_ttl_secs: 900,
            refresh_token_ttl_secs: 604800,
            cookie_domain: None,
            cookie_secure: false,
        });
        AppState {
            store,
            user_store,
            engine: Arc::new(engine),
            jwt_config,
            worker_token: "test-worker-token".to_string(),
        }
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
