//! `POST /api/v1/workflows/:name/pause` and `POST /api/v1/workflows/:name/resume`
//! — Hold back every queued run of a workflow, and release them.

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use ironflow_auth::extractor::Authenticated;
use ironflow_engine::error::EngineError;

use crate::entities::WorkflowPauseResponse;
use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Pause a workflow: workers stop picking its queued runs.
///
/// Runs already executing are left alone (pause them one by one with
/// `POST /api/v1/runs/{id}/pause`), and new runs are still created: they
/// wait in the queue until the workflow is resumed. Pausing a workflow
/// already paused succeeds and keeps its first `paused_at`.
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
    if !auth.is_admin() {
        return Err(ApiError::Forbidden);
    }

    let pause = state
        .engine
        .pause_workflow(&name, Some(auth.user_id))
        .await
        .map_err(|err| map_engine_error(err, &name))?;

    Ok(ok(WorkflowPauseResponse::from(pause)))
}

/// Resume a paused workflow: workers pick its queued runs again.
///
/// Resuming a workflow that is not paused succeeds and changes nothing.
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
    if !auth.is_admin() {
        return Err(ApiError::Forbidden);
    }

    state
        .engine
        .resume_workflow(&name)
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
    use ironflow_auth::jwt::JwtConfig;
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::context::WorkflowContext;
    use ironflow_engine::engine::{Engine, ExecutionMode};
    use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
    use ironflow_engine::notify::Event;
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::store::RunStore;
    use serde_json::{Value as JsonValue, from_slice};
    use tokio::sync::broadcast;
    use tower::ServiceExt;

    use super::*;
    use crate::routes::test_helpers::create_user_auth_header;

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
        let provider = Arc::new(ClaudeCodeProvider::new());
        let mut engine =
            Engine::new(store.clone(), provider).with_execution_mode(ExecutionMode::Workers);
        engine.register(Deploy).unwrap();
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
        assert_eq!(body["data"]["paused"], true);
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
        assert_eq!(body["data"]["paused"], false);
        assert!(body["data"].get("paused_at").is_none());
        assert!(store.list_workflow_pauses().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn resume_workflow_not_paused_is_accepted() {
        let store = Arc::new(InMemoryStore::new());

        let (status, body) = send(test_state(store), "resume", "deploy", true).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["paused"], false);
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
}
