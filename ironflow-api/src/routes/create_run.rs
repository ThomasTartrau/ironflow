//! `POST /api/v1/runs` — Trigger a workflow.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use chrono::Utc;
use ironflow_auth::extractor::Authenticated;
use ironflow_engine::notify::Event;
use ironflow_store::models::TriggerKind;
use serde_json::json;

use crate::actor::run_actor_of;
use crate::entities::{CreateRunRequest, RunResponse};
use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Trigger a workflow by name.
///
/// Returns 201 Created with the newly enqueued run.
/// Returns 400 Bad Request if the workflow is unknown.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        post,
        path = "/api/v1/runs",
        tags = ["runs"],
        request_body(content = CreateRunRequest, description = "Workflow to trigger"),
        responses(
            (status = 201, description = "Run created successfully", body = RunResponse),
            (status = 400, description = "Unknown workflow"),
            (status = 401, description = "Unauthorized"),
            (status = 403, description = "Forbidden")
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

    let payload = req.payload.unwrap_or_else(|| json!({}));
    let labels = req.labels.unwrap_or_default();

    let run = state
        .engine
        .enqueue_handler_with_options(
            &req.workflow,
            TriggerKind::Api,
            payload,
            3,
            labels,
            req.scheduled_at,
            Some(run_actor_of(&auth)),
        )
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

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
    use axum::http::{Request, StatusCode};
    use axum::routing::post;
    use http_body_util::BodyExt;
    use ironflow_auth::extractor::{API_KEY_PREFIX, API_KEY_SUFFIX_LEN};
    use ironflow_auth::jwt::AccessToken;
    use ironflow_auth::password;
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::context::WorkflowContext;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
    use ironflow_engine::notify::Event;
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::models::{ApiKeyScope, NewApiKey, NewUser};
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
        let store = Arc::new(InMemoryStore::new());
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

    // ---- created_by ----

    /// Seed an admin user and return `(user_id, username)`.
    async fn seed_admin(state: &AppState, username: &str) -> (Uuid, String) {
        let user = state
            .store
            .create_user(NewUser {
                email: format!("{username}@example.com"),
                username: username.to_string(),
                password_hash: "hash".to_string(),
                is_admin: Some(true),
            })
            .await
            .expect("create user");
        (user.id, user.username)
    }

    async fn post_create_run(state: AppState, auth_header: &str) -> JsonValue {
        let app = Router::new().route("/", post(create_run)).with_state(state);

        let req = Request::builder()
            .uri("/")
            .method("POST")
            .header("content-type", "application/json")
            .header("authorization", auth_header)
            .body(Body::from(
                serde_json::to_string(&json!({"workflow": "test-workflow"})).unwrap(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&body).unwrap()
    }

    #[tokio::test]
    async fn create_run_records_the_jwt_user_as_author() {
        let state = test_state();
        let (user_id, username) = seed_admin(&state, "alice").await;
        let token = AccessToken::for_user(user_id, &username, true, &state.jwt_config).unwrap();

        let body = post_create_run(state, &format!("Bearer {}", token.0)).await;

        assert_eq!(body["data"]["created_by"]["kind"], "user");
        assert_eq!(body["data"]["created_by"]["id"], user_id.to_string());
        assert_eq!(body["data"]["created_by"]["label"], "alice");
    }

    #[tokio::test]
    async fn create_run_records_the_api_key_as_author() {
        let state = test_state();
        let (user_id, _) = seed_admin(&state, "alice").await;

        let raw_key = format!("{API_KEY_PREFIX}0123456789abcdef");
        let key = state
            .store
            .create_api_key(NewApiKey {
                user_id,
                name: "ci-deploy".to_string(),
                key_hash: password::hash(&raw_key).unwrap(),
                key_prefix: raw_key[..API_KEY_PREFIX.len() + API_KEY_SUFFIX_LEN].to_string(),
                scopes: vec![ApiKeyScope::RunsWrite],
                expires_at: None,
            })
            .await
            .expect("create api key");

        let body = post_create_run(state, &format!("Bearer {raw_key}")).await;

        assert_eq!(body["data"]["created_by"]["kind"], "api_key");
        assert_eq!(body["data"]["created_by"]["id"], key.id.to_string());
        assert_eq!(body["data"]["created_by"]["label"], "ci-deploy (alice)");
    }

    #[tokio::test]
    async fn create_run_author_label_falls_back_when_the_user_is_unknown() {
        // A valid token for a user that is not in the store (e.g. deleted).
        let state = test_state();
        let auth_header = make_auth_header(&state);

        let body = post_create_run(state, &auth_header).await;

        assert_eq!(body["data"]["created_by"]["kind"], "user");
        let label = body["data"]["created_by"]["label"].as_str().unwrap();
        assert!(label.starts_with("user "), "unexpected label: {label}");
    }
}
