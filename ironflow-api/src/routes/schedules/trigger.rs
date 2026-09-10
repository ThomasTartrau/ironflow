//! `POST /api/v1/schedules/{id}/trigger` -- Trigger a schedule manually.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use uuid::Uuid;

use ironflow_auth::extractor::Authenticated;
use ironflow_store::entities::RunActor;

use crate::entities::ScheduleResponse;
use crate::error::ApiError;
use crate::response::ok;
use crate::schedule_ticker::new_run_from_schedule;
use crate::state::AppState;

/// Trigger a schedule manually, creating a run immediately.
///
/// # Errors
///
/// - 401 if not authenticated
/// - 404 if the schedule does not exist
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        post,
        path = "/api/v1/schedules/{id}/trigger",
        tags = ["schedules"],
        params(("id" = Uuid, Path, description = "Schedule ID")),
        responses(
            (status = 201, description = "Run created from schedule", body = ScheduleResponse),
            (status = 401, description = "Unauthorized"),
            (status = 404, description = "Schedule not found")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn trigger_schedule(
    auth: Authenticated,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let schedule = state
        .store
        .find_schedule_by_id(id)
        .await?
        .ok_or(ApiError::ScheduleNotFound(id))?;

    let _run = state
        .store
        .create_run(new_run_from_schedule(
            &schedule,
            Some(RunActor::User {
                user_id: auth.user_id,
            }),
        ))
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
    use ironflow_store::entities::{NewSchedule, NewUser, RunFilter, ScheduleSource};
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
            secret: "test-secret-trigger".to_string(),
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
                inputs: json!({"env": "prod"}),
                source: ScheduleSource::Api,
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
    async fn trigger_creates_run() {
        let (state, user_id, schedule_id) = test_state_with_schedule().await;
        let auth = make_auth_header(user_id, &state);
        let app = Router::new()
            .route("/{id}/trigger", post(trigger_schedule))
            .with_state(state.clone());

        let req = Request::builder()
            .uri(format!("/{schedule_id}/trigger"))
            .method("POST")
            .header("authorization", &auth)
            .body(Body::empty())
            .expect("build");

        let resp = app.oneshot(req).await.expect("request");
        assert_eq!(resp.status(), StatusCode::CREATED);

        let body = resp.into_body().collect().await.expect("body").to_bytes();
        let val: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(val["data"]["workflow_name"], "deploy");

        let runs = state
            .store
            .list_runs(RunFilter::default(), 0, 10)
            .await
            .expect("list runs");
        assert_eq!(runs.items.len(), 1);
        assert_eq!(runs.items[0].workflow_name, "deploy");
    }
}
