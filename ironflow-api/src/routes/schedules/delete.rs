//! `DELETE /api/v1/schedules/{id}` -- Delete a schedule.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use uuid::Uuid;

use ironflow_auth::extractor::Authenticated;
use ironflow_store::entities::ScheduleSource;

use crate::error::ApiError;
use crate::state::AppState;

/// Delete a schedule by ID.
///
/// Handler-declared schedules (`source = handler`) cannot be deleted via the
/// API -- they are managed by the code and reconciled at startup.
///
/// # Errors
///
/// - 401 if not authenticated
/// - 403 if the schedule is handler-declared
/// - 404 if the schedule does not exist
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        delete,
        path = "/api/v1/schedules/{id}",
        tags = ["schedules"],
        params(("id" = Uuid, Path, description = "Schedule ID")),
        responses(
            (status = 204, description = "Schedule deleted"),
            (status = 401, description = "Unauthorized"),
            (status = 403, description = "Cannot delete handler-declared schedule"),
            (status = 404, description = "Schedule not found")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn delete_schedule(
    _auth: Authenticated,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let schedule = state
        .store
        .find_schedule_by_id(id)
        .await?
        .ok_or(ApiError::ScheduleNotFound(id))?;

    if schedule.source == ScheduleSource::Handler {
        return Err(ApiError::Conflict(
            "cannot delete a handler-declared schedule; remove it from the code instead"
                .to_string(),
        ));
    }

    state.store.delete_schedule(id).await?;

    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::delete;
    use ironflow_auth::jwt::{AccessToken, JwtConfig};
    use ironflow_auth::password;
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::context::WorkflowContext;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
    use ironflow_engine::notify::Event;
    use ironflow_store::entities::{NewSchedule, NewUser, ScheduleSource};
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
            secret: "test-secret-for-schedule-delete".to_string(),
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
    async fn delete_existing_schedule() {
        let (state, user_id) = test_state_with_user().await;
        let schedule = state
            .store
            .create_schedule(NewSchedule {
                workflow_name: "deploy".to_string(),
                cron_expression: "0 0 * * * *".to_string(),
                inputs: json!({}),
                source: ScheduleSource::Api,
                created_by_user_id: Some(user_id),
                next_trigger_at: None,
            })
            .await
            .expect("create");

        let auth = make_auth_header(user_id, &state);
        let app = Router::new()
            .route("/{id}", delete(delete_schedule))
            .with_state(state);

        let req = Request::builder()
            .uri(format!("/{}", schedule.id))
            .method("DELETE")
            .header("authorization", &auth)
            .body(Body::empty())
            .expect("build");

        let resp = app.oneshot(req).await.expect("request");
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn delete_nonexistent_schedule() {
        let (state, user_id) = test_state_with_user().await;
        let auth = make_auth_header(user_id, &state);
        let app = Router::new()
            .route("/{id}", delete(delete_schedule))
            .with_state(state);

        let fake_id = Uuid::now_v7();
        let req = Request::builder()
            .uri(format!("/{fake_id}"))
            .method("DELETE")
            .header("authorization", &auth)
            .body(Body::empty())
            .expect("build");

        let resp = app.oneshot(req).await.expect("request");
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
