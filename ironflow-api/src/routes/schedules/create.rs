//! `POST /api/v1/schedules` -- Create a new schedule.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use validator::Validate;

use ironflow_auth::extractor::Authenticated;
use ironflow_store::entities::NewSchedule;

use crate::entities::{CreateScheduleRequest, ScheduleResponse};
use crate::error::ApiError;
use crate::response::ok;
use crate::schedule_ticker::next_trigger;
use crate::state::AppState;

/// Create a new schedule.
///
/// # Errors
///
/// - 400 if validation fails or cron expression is invalid
/// - 400 if the workflow is not registered
/// - 401 if not authenticated
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        post,
        path = "/api/v1/schedules",
        tags = ["schedules"],
        request_body(content = CreateScheduleRequest, description = "Schedule definition"),
        responses(
            (status = 201, description = "Schedule created", body = ScheduleResponse),
            (status = 400, description = "Invalid input"),
            (status = 401, description = "Unauthorized")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn create_schedule(
    auth: Authenticated,
    State(state): State<AppState>,
    Json(req): Json<CreateScheduleRequest>,
) -> Result<impl IntoResponse, ApiError> {
    req.validate()
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    if state.engine.get_handler(&req.workflow_name).is_none() {
        return Err(ApiError::BadRequest(format!(
            "workflow '{}' is not registered",
            req.workflow_name
        )));
    }

    let next = next_trigger(&req.cron_expression).map_err(ApiError::BadRequest)?;

    let schedule = state
        .store
        .create_schedule(NewSchedule {
            workflow_name: req.workflow_name,
            cron_expression: req.cron_expression,
            inputs: req.inputs,
            created_by_user_id: auth.user_id,
            next_trigger_at: next,
        })
        .await?;

    Ok((StatusCode::CREATED, ok(ScheduleResponse::from(schedule))))
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::post;
    use http_body_util::BodyExt;
    use ironflow_auth::jwt::{AccessToken, JwtConfig};
    use ironflow_auth::password;
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::context::WorkflowContext;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
    use ironflow_engine::notify::Event;
    use ironflow_store::entities::NewUser;
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::store::Store;
    use serde_json::json;
    use std::sync::Arc;
    use tokio::sync::broadcast;
    use tower::ServiceExt;
    use uuid::Uuid;

    use crate::state::AppState;

    use super::*;

    struct TestWorkflow;

    impl WorkflowHandler for TestWorkflow {
        fn name(&self) -> &str {
            "deploy"
        }

        fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
            Box::pin(async move { Ok(()) })
        }
    }

    fn test_jwt_config() -> Arc<JwtConfig> {
        Arc::new(JwtConfig {
            secret: "test-secret-for-schedule-tests".to_string(),
            access_token_ttl_secs: 900,
            refresh_token_ttl_secs: 604800,
            cookie_domain: None,
            cookie_secure: false,
        })
    }

    async fn test_state_with_user() -> (AppState, Uuid) {
        let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
        let provider = Arc::new(ClaudeCodeProvider::new());
        let mut engine = Engine::new(store.clone(), provider);
        engine.register(TestWorkflow).expect("register");
        let (event_sender, _) = broadcast::channel::<Event>(1);
        let state = AppState::new(
            store.clone(),
            Arc::new(engine),
            test_jwt_config(),
            "test-worker-token".to_string(),
            event_sender,
        );
        let hash = password::hash("password123").expect("hash");
        let user = store
            .create_user(NewUser {
                email: "test@example.com".to_string(),
                username: "testuser".to_string(),
                password_hash: hash,
                is_admin: None,
            })
            .await
            .expect("create user");
        (state, user.id)
    }

    fn make_auth_header(user_id: Uuid, state: &AppState) -> String {
        let token =
            AccessToken::for_user(user_id, "testuser", false, &state.jwt_config).expect("token");
        format!("Bearer {}", token.0)
    }

    #[tokio::test]
    async fn create_schedule_success() {
        let (state, user_id) = test_state_with_user().await;
        let auth = make_auth_header(user_id, &state);
        let app = Router::new()
            .route("/", post(create_schedule))
            .with_state(state);

        let req = Request::builder()
            .uri("/")
            .method("POST")
            .header("authorization", &auth)
            .header("content-type", "application/json")
            .body(Body::from(
                json!({
                    "workflow_name": "deploy",
                    "cron_expression": "0 0 * * * *",
                    "inputs": {"env": "prod"}
                })
                .to_string(),
            ))
            .expect("build");

        let resp = app.oneshot(req).await.expect("request");
        assert_eq!(resp.status(), StatusCode::CREATED);

        let body = resp.into_body().collect().await.expect("body").to_bytes();
        let val: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(val["data"]["workflow_name"], "deploy");
        assert!(val["data"]["disabled_at"].is_null());
        assert!(val["data"]["next_trigger_at"].is_string());
    }

    #[tokio::test]
    async fn create_schedule_invalid_cron() {
        let (state, user_id) = test_state_with_user().await;
        let auth = make_auth_header(user_id, &state);
        let app = Router::new()
            .route("/", post(create_schedule))
            .with_state(state);

        let req = Request::builder()
            .uri("/")
            .method("POST")
            .header("authorization", &auth)
            .header("content-type", "application/json")
            .body(Body::from(
                json!({
                    "workflow_name": "deploy",
                    "cron_expression": "not-a-cron",
                })
                .to_string(),
            ))
            .expect("build");

        let resp = app.oneshot(req).await.expect("request");
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn create_schedule_unknown_workflow() {
        let (state, user_id) = test_state_with_user().await;
        let auth = make_auth_header(user_id, &state);
        let app = Router::new()
            .route("/", post(create_schedule))
            .with_state(state);

        let req = Request::builder()
            .uri("/")
            .method("POST")
            .header("authorization", &auth)
            .header("content-type", "application/json")
            .body(Body::from(
                json!({
                    "workflow_name": "nonexistent",
                    "cron_expression": "0 0 * * * *",
                })
                .to_string(),
            ))
            .expect("build");

        let resp = app.oneshot(req).await.expect("request");
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let body = resp.into_body().collect().await.expect("body").to_bytes();
        let val: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert!(
            val["error"]["message"]
                .as_str()
                .expect("msg")
                .contains("not registered")
        );
    }

    #[tokio::test]
    async fn create_schedule_unauthenticated() {
        let (state, _) = test_state_with_user().await;
        let app = Router::new()
            .route("/", post(create_schedule))
            .with_state(state);

        let req = Request::builder()
            .uri("/")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({
                    "workflow_name": "deploy",
                    "cron_expression": "0 0 * * * *",
                })
                .to_string(),
            ))
            .expect("build");

        let resp = app.oneshot(req).await.expect("request");
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}
