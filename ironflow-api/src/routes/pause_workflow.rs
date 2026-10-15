//! `POST /api/v1/workflows/:name/pause` and `POST /api/v1/workflows/:name/resume`
//! — Hold back every queued run of a workflow, and release them.

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use ironflow_auth::extractor::Authenticated;
use ironflow_engine::error::EngineError;
use ironflow_store::entities::ApiKeyScope;

use crate::authorization::AuthorizeExt;
use crate::entities::WorkflowPauseResponse;
use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Pause a workflow: workers stop picking its queued runs.
///
/// Runs already executing are left alone (pause them one by one with
/// `POST /api/v1/runs/{id}/pause`), and new runs are still created: they
/// wait in the queue until the workflow is resumed. Pausing a workflow
/// already paused succeeds and keeps its first `paused_at`. Every pause
/// publishes a `workflow_paused` audit event carrying the caller's user id.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        post,
        path = "/api/v1/workflows/{name}/pause",
        tags = ["workflows"],
        params(("name" = String, Path, description = "Workflow name")),
        responses(
            (status = 200, description = "Workflow paused", body = WorkflowPauseResponse),
            (status = 401, description = "Unauthorized"),
            (status = 403, description = "Forbidden"),
            (status = 404, description = "Workflow not found")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn pause_workflow(
    auth: Authenticated,
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    auth.require(ApiKeyScope::Admin, true)?;

    let pause = state
        .engine
        .pause_workflow(&name, Some(auth.user_id))
        .await
        .map_err(|err| map_engine_error(err, &name))?;

    Ok(ok(WorkflowPauseResponse::from(pause)))
}

/// Resume a paused workflow: workers pick its queued runs again.
///
/// Resuming a workflow that is not paused succeeds and changes nothing. A
/// resume of a paused workflow publishes a `workflow_resumed` audit event
/// carrying the caller's user id.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        post,
        path = "/api/v1/workflows/{name}/resume",
        tags = ["workflows"],
        params(("name" = String, Path, description = "Workflow name")),
        responses(
            (status = 200, description = "Workflow resumed", body = WorkflowPauseResponse),
            (status = 401, description = "Unauthorized"),
            (status = 403, description = "Forbidden"),
            (status = 404, description = "Workflow not found")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn resume_workflow(
    auth: Authenticated,
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    auth.require(ApiKeyScope::Admin, true)?;

    state
        .engine
        .resume_workflow(&name, Some(auth.user_id))
        .await
        .map_err(|err| map_engine_error(err, &name))?;

    Ok(ok(WorkflowPauseResponse::resumed(&name)))
}

/// Map an error of a workflow pause or resume to its HTTP status.
fn map_engine_error(err: EngineError, name: &str) -> ApiError {
    match err {
        EngineError::InvalidWorkflow(_) => ApiError::WorkflowNotFound(name.to_string()),
        EngineError::Store(err) => ApiError::from(err),
        other => ApiError::Internal(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::post;
    use http_body_util::BodyExt;
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::context::WorkflowContext;
    use ironflow_engine::engine::{Engine, ExecutionMode};
    use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
    use ironflow_store::entities::EventKind;
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::store::RunStore;
    use ironflow_store::user_store::UserStore;
    use serde_json::{Value as JsonValue, from_slice};
    use tower::ServiceExt;

    use super::*;
    use crate::routes::test_helpers::{
        await_audit_entry, create_user_auth_header, state_with_engine,
    };

    struct Deploy;

    impl WorkflowHandler for Deploy {
        fn name(&self) -> &str {
            "deploy"
        }

        fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
            Box::pin(async move { Ok(()) })
        }
    }

    fn test_state(store: Arc<InMemoryStore>) -> AppState {
        state_with(store, false)
    }

    /// An `AppState` whose engine runs `deploy`, persisting every event to the
    /// store's audit log when `audit_log` is set.
    fn state_with(store: Arc<InMemoryStore>, audit_log: bool) -> AppState {
        let provider = Arc::new(ClaudeCodeProvider::new());
        let mut engine =
            Engine::new(store.clone(), provider).with_execution_mode(ExecutionMode::Workers);
        engine.register(Deploy).unwrap();
        state_with_engine(store, engine, audit_log)
    }

    async fn send(
        state: AppState,
        action: &str,
        name: &str,
        admin: bool,
    ) -> (StatusCode, JsonValue) {
        let auth_header = create_user_auth_header(&state, "testuser", admin).await;
        let app = Router::new()
            .route("/{name}/pause", post(pause_workflow))
            .route("/{name}/resume", post(resume_workflow))
            .with_state(state);

        let req = Request::builder()
            .method("POST")
            .uri(format!("/{name}/{action}"))
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        (status, from_slice(&body).unwrap())
    }

    #[tokio::test]
    async fn pause_workflow_returns_paused() {
        let store = Arc::new(InMemoryStore::new());

        let (status, body) = send(test_state(store.clone()), "pause", "deploy", true).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["workflow_name"], "deploy");
        assert!(body["data"]["paused_at"].is_string());
        assert!(body["data"]["paused_by"].is_string());
        let pauses = store.list_workflow_pauses().await.unwrap();
        assert_eq!(pauses.len(), 1);
        assert_eq!(pauses[0].workflow_name, "deploy");
    }

    #[tokio::test]
    async fn pause_unknown_workflow_returns_404() {
        let store = Arc::new(InMemoryStore::new());

        let (status, _) = send(test_state(store.clone()), "pause", "unknown", true).await;

        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(store.list_workflow_pauses().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn pause_workflow_non_admin_returns_403() {
        let store = Arc::new(InMemoryStore::new());

        let (status, _) = send(test_state(store.clone()), "pause", "deploy", false).await;

        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(store.list_workflow_pauses().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn resume_workflow_returns_not_paused() {
        let store = Arc::new(InMemoryStore::new());
        store.pause_workflow("deploy", None).await.unwrap();

        let (status, body) = send(test_state(store.clone()), "resume", "deploy", true).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["workflow_name"], "deploy");
        assert!(body["data"].get("paused_at").is_none());
        assert!(store.list_workflow_pauses().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn resume_workflow_not_paused_is_accepted() {
        let store = Arc::new(InMemoryStore::new());

        let (status, body) = send(test_state(store), "resume", "deploy", true).await;

        assert_eq!(status, StatusCode::OK);
        assert!(body["data"].get("paused_at").is_none());
    }

    #[tokio::test]
    async fn resume_unknown_workflow_returns_404() {
        let store = Arc::new(InMemoryStore::new());

        let (status, _) = send(test_state(store), "resume", "unknown", true).await;

        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn resume_workflow_non_admin_returns_403() {
        let store = Arc::new(InMemoryStore::new());
        store.pause_workflow("deploy", None).await.unwrap();

        let (status, _) = send(test_state(store.clone()), "resume", "deploy", false).await;

        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(store.list_workflow_pauses().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn pause_and_resume_are_recorded_in_the_audit_log_with_the_admin() {
        let store = Arc::new(InMemoryStore::new());
        let state = state_with(store.clone(), true);

        let (status, _) = send(state.clone(), "pause", "deploy", true).await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = send(state, "resume", "deploy", true).await;
        assert_eq!(status, StatusCode::OK);

        let admin = store
            .find_user_by_username("testuser")
            .await
            .unwrap()
            .expect("admin created by send");
        let paused = await_audit_entry(&store, EventKind::WorkflowPaused, None).await;
        assert_eq!(paused.user_id, Some(admin.id));
        assert_eq!(paused.payload["workflow_name"], "deploy");
        let resumed = await_audit_entry(&store, EventKind::WorkflowResumed, None).await;
        assert_eq!(resumed.user_id, Some(admin.id));
        assert_eq!(resumed.payload["user_id"], admin.id.to_string());
    }
}
