//! `POST /api/v1/schedules/{id}/pause` and `POST /api/v1/schedules/{id}/resume`.

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use uuid::Uuid;

use ironflow_auth::extractor::Authenticated;
use ironflow_store::entities::ScheduleUpdate;

use crate::entities::ScheduleResponse;
use crate::error::ApiError;
use crate::response::ok;
use crate::schedule_ticker::next_trigger;
use crate::state::AppState;

/// Pause a schedule (set enabled = false).
///
/// # Errors
///
/// - 401 if not authenticated
/// - 404 if the schedule does not exist
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        post,
        path = "/api/v1/schedules/{id}/pause",
        tags = ["schedules"],
        params(("id" = Uuid, Path, description = "Schedule ID")),
        responses(
            (status = 200, description = "Schedule paused", body = ScheduleResponse),
            (status = 401, description = "Unauthorized"),
            (status = 404, description = "Schedule not found")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn pause_schedule(
    _auth: Authenticated,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let schedule = state
        .store
        .update_schedule(
            id,
            ScheduleUpdate {
                disabled_at: Some(Some(chrono::Utc::now())),
                ..Default::default()
            },
        )
        .await?;

    Ok(ok(ScheduleResponse::from(schedule)))
}

/// Resume a schedule (clear disabled_at).
///
/// # Errors
///
/// - 401 if not authenticated
/// - 404 if the schedule does not exist
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        post,
        path = "/api/v1/schedules/{id}/resume",
        tags = ["schedules"],
        params(("id" = Uuid, Path, description = "Schedule ID")),
        responses(
            (status = 200, description = "Schedule resumed", body = ScheduleResponse),
            (status = 401, description = "Unauthorized"),
            (status = 404, description = "Schedule not found")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn resume_schedule(
    _auth: Authenticated,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let current = state
        .store
        .find_schedule_by_id(id)
        .await?
        .ok_or(ApiError::ScheduleNotFound(id))?;

    let next = next_trigger(&current.cron_expression).map_err(ApiError::BadRequest)?;

    let schedule = state
        .store
        .update_schedule(
            id,
            ScheduleUpdate {
                disabled_at: Some(None),
                next_trigger_at: Some(next),
                ..Default::default()
            },
        )
        .await?;

    Ok(ok(ScheduleResponse::from(schedule)))
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
    use ironflow_store::entities::{NewSchedule, NewUser};
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
            secret: "test-secret-pause-resume".to_string(),
            access_token_ttl_secs: 900,
            refresh_token_ttl_secs: 604800,
            cookie_domain: None,
            cookie_secure: false,
        })
    }

    async fn test_state_with_schedule() -> (AppState, Uuid, Uuid) {
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
        let schedule = store
            .create_schedule(NewSchedule {
                workflow_name: "deploy".to_string(),
                cron_expression: "0 0 * * * *".to_string(),
                inputs: json!({}),
                created_by_user_id: user.id,
                next_trigger_at: None,
            })
            .await
            .expect("create schedule");
        (state, user.id, schedule.id)
    }

    fn make_auth_header(user_id: Uuid, state: &AppState) -> String {
        let token =
            AccessToken::for_user(user_id, "testuser", false, &state.jwt_config).expect("token");
        format!("Bearer {}", token.0)
    }

    #[tokio::test]
    async fn pause_and_resume() {
        let (state, user_id, schedule_id) = test_state_with_schedule().await;
        let auth = make_auth_header(user_id, &state);
        let app = Router::new()
            .route("/{id}/pause", post(pause_schedule))
            .route("/{id}/resume", post(resume_schedule))
            .with_state(state);

        let req = Request::builder()
            .uri(format!("/{schedule_id}/pause"))
            .method("POST")
            .header("authorization", &auth)
            .body(Body::empty())
            .expect("build");

        let resp = app.clone().oneshot(req).await.expect("request");
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.expect("body").to_bytes();
        let val: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert!(val["data"]["disabled_at"].is_string());

        let req = Request::builder()
            .uri(format!("/{schedule_id}/resume"))
            .method("POST")
            .header("authorization", &auth)
            .body(Body::empty())
            .expect("build");

        let resp = app.oneshot(req).await.expect("request");
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.expect("body").to_bytes();
        let val: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert!(val["data"]["disabled_at"].is_null());
    }
}
