//! `POST /api/v1/schedules` -- Create a new schedule.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use validator::Validate;

use ironflow_auth::extractor::Authenticated;
use ironflow_store::entities::{
    MAX_PRIORITY, MIN_PRIORITY, NewSchedule, SchedulePolicy, ScheduleSource, validate_priority,
};

use crate::entities::{CreateScheduleRequest, ScheduleResponse};
use crate::error::ApiError;
use crate::response::ok;
use crate::schedule_clock::{next_trigger, parse_timezone};
use crate::state::AppState;

/// Create a new schedule.
///
/// # Errors
///
/// - 400 if validation fails or cron expression is invalid
/// - 400 if `priority` is outside `-100..=100`
/// - 400 if `catchup_max` is outside `1..=1000` or `catchup_window_secs` is
///   outside `60..=2592000`
/// - 400 if `timezone` is not an IANA timezone name
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
            (status = 400, description = "Invalid input, cron, priority, catch-up or timezone"),
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

    if let Some(priority) = req.priority {
        validate_priority(priority).map_err(ApiError::BadRequest)?;
    }

    let Some(handler) = state.engine.get_handler(&req.workflow_name) else {
        return Err(ApiError::BadRequest(format!(
            "workflow '{}' is not registered",
            req.workflow_name
        )));
    };
    let priority = req
        .priority
        .unwrap_or_else(|| handler.priority().clamp(MIN_PRIORITY, MAX_PRIORITY));

    let defaults = SchedulePolicy::default();
    let mut policy = SchedulePolicy {
        catchup: req.catchup.unwrap_or(defaults.catchup),
        catchup_max: req.catchup_max.unwrap_or(defaults.catchup_max),
        catchup_window_secs: req
            .catchup_window_secs
            .unwrap_or(defaults.catchup_window_secs),
        overlap: req.overlap.unwrap_or(defaults.overlap),
        timezone: defaults.timezone,
    };
    policy.validate().map_err(ApiError::BadRequest)?;
    if let Some(timezone) = &req.timezone {
        // Store the canonical name, as the engine builder does.
        policy.timezone = parse_timezone(timezone).map_err(ApiError::BadRequest)?;
    }

    let next = next_trigger(&req.cron_expression, policy.timezone).map_err(ApiError::BadRequest)?;

    let schedule = state
        .store
        .create_schedule(NewSchedule {
            workflow_name: req.workflow_name,
            cron_expression: req.cron_expression,
            inputs: req.inputs,
            source: ScheduleSource::Api,
            priority,
            created_by_user_id: Some(auth.user_id),
            next_trigger_at: next,
            policy,
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
    use serde_json::{Value, from_slice, json};
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

    struct UrgentWorkflow;

    impl WorkflowHandler for UrgentWorkflow {
        fn name(&self) -> &str {
            "urgent"
        }

        fn priority(&self) -> i16 {
            25
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
        engine.register(UrgentWorkflow).expect("register");
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

    async fn post_schedule(body: Value) -> (StatusCode, Value) {
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
            .body(Body::from(body.to_string()))
            .expect("build");

        let resp = app.oneshot(req).await.expect("request");
        let status = resp.status();
        let bytes = resp.into_body().collect().await.expect("body").to_bytes();
        (status, from_slice(&bytes).expect("json"))
    }

    #[tokio::test]
    async fn create_schedule_priority_defaults_to_the_handler_priority() {
        let (status, val) = post_schedule(json!({
            "workflow_name": "urgent",
            "cron_expression": "0 0 * * * *",
        }))
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(val["data"]["priority"], 25);

        let (status, val) = post_schedule(json!({
            "workflow_name": "deploy",
            "cron_expression": "0 0 * * * *",
        }))
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(val["data"]["priority"], 0);
    }

    #[tokio::test]
    async fn create_schedule_priority_explicit_value_overrides_the_handler() {
        let (status, val) = post_schedule(json!({
            "workflow_name": "urgent",
            "cron_expression": "0 0 * * * *",
            "priority": -70,
        }))
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(val["data"]["priority"], -70);
    }

    #[tokio::test]
    async fn create_schedule_priority_out_of_range_returns_400() {
        for priority in [MAX_PRIORITY + 1, MIN_PRIORITY - 1] {
            let (status, val) = post_schedule(json!({
                "workflow_name": "deploy",
                "cron_expression": "0 0 * * * *",
                "priority": priority,
            }))
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert_eq!(
                val["error"]["message"],
                "priority must be between -100 and 100"
            );
        }
    }

    #[tokio::test]
    async fn create_schedule_with_catchup_policies_and_timezone() {
        let (status, val) = post_schedule(json!({
            "workflow_name": "deploy",
            "cron_expression": "0 9 * * *",
            "catchup": "all",
            "catchup_max": 24,
            "catchup_window_secs": 3600,
            "overlap": "skip",
            "timezone": "Europe/Paris",
        }))
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let data = &val["data"];
        assert_eq!(data["catchup"], "all");
        assert_eq!(data["catchup_max"], 24);
        assert_eq!(data["catchup_window_secs"], 3600);
        assert_eq!(data["overlap"], "skip");
        assert_eq!(data["timezone"], "Europe/Paris");
        // 9:00 in Paris is 7:00 or 8:00 UTC depending on DST, never 9:00.
        let next = data["next_trigger_at"].as_str().expect("next trigger");
        assert!(
            next.contains("T07:00:00") || next.contains("T08:00:00"),
            "{next}"
        );
    }

    #[tokio::test]
    async fn create_schedule_defaults_to_latest_allow_utc() {
        let (status, val) = post_schedule(json!({
            "workflow_name": "deploy",
            "cron_expression": "0 0 * * * *",
        }))
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let data = &val["data"];
        assert_eq!(data["catchup"], "latest");
        assert_eq!(data["catchup_max"], 10);
        assert_eq!(data["catchup_window_secs"], 86400);
        assert_eq!(data["overlap"], "allow");
        assert_eq!(data["timezone"], "UTC");
    }

    #[tokio::test]
    async fn create_schedule_rejects_invalid_timezone() {
        let (status, val) = post_schedule(json!({
            "workflow_name": "deploy",
            "cron_expression": "0 9 * * *",
            "timezone": "Mars/Olympus",
        }))
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let message = val["error"]["message"].as_str().expect("msg");
        assert!(
            message.contains("invalid timezone 'Mars/Olympus'"),
            "{message}"
        );
    }

    #[tokio::test]
    async fn create_schedule_rejects_catchup_max_out_of_range() {
        for catchup_max in [0, 1001] {
            let (status, val) = post_schedule(json!({
                "workflow_name": "deploy",
                "cron_expression": "0 9 * * *",
                "catchup_max": catchup_max,
            }))
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            let message = val["error"]["message"].as_str().expect("msg");
            assert!(message.contains("catchup_max"), "{message}");
        }
    }

    #[tokio::test]
    async fn create_schedule_rejects_catchup_window_below_a_minute() {
        let (status, val) = post_schedule(json!({
            "workflow_name": "deploy",
            "cron_expression": "0 9 * * *",
            "catchup_window_secs": 59,
        }))
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let message = val["error"]["message"].as_str().expect("msg");
        assert!(message.contains("catchup_window"), "{message}");
    }

    #[tokio::test]
    async fn create_schedule_rejects_unknown_catchup_value() {
        let (status, _) = post_schedule(json!({
            "workflow_name": "deploy",
            "cron_expression": "0 9 * * *",
            "catchup": "sometimes",
        }))
        .await;
        assert!(status.is_client_error(), "{status}");
    }
}
