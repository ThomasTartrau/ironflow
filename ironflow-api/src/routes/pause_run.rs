//! `POST /api/v1/runs/:id/pause` and `POST /api/v1/runs/:id/resume` — Pause a
//! run with the sub-workflow runs below it, and resume it.

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use ironflow_auth::extractor::Authenticated;
use ironflow_engine::error::EngineError;
use ironflow_store::error::StoreError;
use uuid::Uuid;

use crate::entities::{PauseRunResponse, ResumeRunResponse, RunResponse};
use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Pause a run that has not finished, with every sub-workflow run below it.
///
/// A queued, sleeping or waiting run is no longer picked or woken. A running
/// run has its step in flight interrupted and starts no further step; the
/// resume executes the interrupted step again (see
/// [`Engine::pause_run`](ironflow_engine::engine::Engine::pause_run)). The
/// run keeps the state it was paused from in `resume_status`.
///
/// Returns 400 for a run that already finished, a run already paused, or a
/// sub-workflow run: pause its root run instead.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        post,
        path = "/api/v1/runs/{id}/pause",
        tags = ["runs"],
        params(("id" = Uuid, Path, description = "Run ID")),
        responses(
            (status = 200, description = "Run paused, with the sub-runs paused along", body = PauseRunResponse),
            (status = 400, description = "Run cannot be paused"),
            (status = 401, description = "Unauthorized"),
            (status = 403, description = "Forbidden"),
            (status = 404, description = "Run not found")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn pause_run(
    auth: Authenticated,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    if !auth.is_admin() {
        return Err(ApiError::Forbidden);
    }

    let pause = state
        .engine
        .pause_run(id)
        .await
        .map_err(|err| map_engine_error(err, "pause"))?;

    Ok(ok(PauseRunResponse {
        run: RunResponse::from(pause.run),
        paused_descendants: pause.paused_descendants,
    }))
}

/// Resume a paused run, with the sub-workflow runs paused below it.
///
/// The run returns to the state it was paused from: a queued run is picked
/// again, a sleeping run waits for its deadline, a run waiting for an
/// approval keeps waiting. A run paused while it executed goes back to
/// `pending` and replays from the step where it stopped.
///
/// Returns 400 for a run that is not paused or a sub-workflow run.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        post,
        path = "/api/v1/runs/{id}/resume",
        tags = ["runs"],
        params(("id" = Uuid, Path, description = "Run ID")),
        responses(
            (status = 200, description = "Run resumed, with the sub-runs resumed along", body = ResumeRunResponse),
            (status = 400, description = "Run is not paused"),
            (status = 401, description = "Unauthorized"),
            (status = 403, description = "Forbidden"),
            (status = 404, description = "Run not found")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn resume_run(
    auth: Authenticated,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    if !auth.is_admin() {
        return Err(ApiError::Forbidden);
    }

    let resume = state
        .engine
        .resume_paused_run(id)
        .await
        .map_err(|err| map_engine_error(err, "resume"))?;

    Ok(ok(ResumeRunResponse {
        run: RunResponse::from(resume.run),
        resumed_descendants: resume.resumed_descendants,
    }))
}

/// Map an error of a pause or a resume to its HTTP status.
fn map_engine_error(err: EngineError, action: &str) -> ApiError {
    match err {
        EngineError::Store(StoreError::RunNotFound(id)) => ApiError::RunNotFound(id),
        EngineError::Store(StoreError::InvalidTransition { from, .. }) => {
            ApiError::BadRequest(format!("cannot {action} run in {from} state"))
        }
        EngineError::ChildRunNotPausable { .. } => ApiError::BadRequest(err.to_string()),
        EngineError::Store(err) => ApiError::from(err),
        other => ApiError::Internal(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::post;
    use http_body_util::BodyExt;
    use ironflow_auth::jwt::JwtConfig;
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::engine::{Engine, ExecutionMode};
    use ironflow_engine::notify::Event;
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::models::{NewRun, Run, RunStatus, TriggerKind};
    use ironflow_store::store::RunStore;
    use serde_json::{Value as JsonValue, from_slice, json};
    use tokio::sync::broadcast;
    use tower::ServiceExt;

    use super::*;
    use crate::routes::test_helpers::create_user_auth_header;

    fn test_state(store: Arc<InMemoryStore>) -> AppState {
        let provider = Arc::new(ClaudeCodeProvider::new());
        let engine =
            Engine::new(store.clone(), provider).with_execution_mode(ExecutionMode::Workers);
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

    async fn create_run(store: &InMemoryStore) -> Run {
        store
            .create_run(NewRun {
                created_by: None,
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 0,
                handler_version: None,
                labels: HashMap::new(),
                scheduled_at: None,
                idempotency_key: None,
                concurrency_key: None,
                priority: 0,
                concurrency_limits: Vec::new(),
                max_cost_usd: None,
                worker_tags: Vec::new(),
            })
            .await
            .unwrap()
            .into_run()
    }

    async fn send(state: AppState, action: &str, id: Uuid, admin: bool) -> (StatusCode, JsonValue) {
        let auth_header = create_user_auth_header(&state, "testuser", admin).await;
        let app = Router::new()
            .route("/{id}/pause", post(pause_run))
            .route("/{id}/resume", post(resume_run))
            .with_state(state);

        let req = Request::builder()
            .method("POST")
            .uri(format!("/{id}/{action}"))
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        (status, from_slice(&body).unwrap())
    }

    #[tokio::test]
    async fn pause_run_returns_paused_status() {
        let store = Arc::new(InMemoryStore::new());
        let run = create_run(&store).await;

        let (status, body) = send(test_state(store.clone()), "pause", run.id, true).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["status"], "paused");
        assert_eq!(body["data"]["resume_status"], "pending");
        assert_eq!(body["data"]["paused_descendants"], json!([]));
        let paused = store.get_run(run.id).await.unwrap().unwrap();
        assert_eq!(paused.status.state, RunStatus::Paused);
    }

    #[tokio::test]
    async fn pause_completed_run_returns_400() {
        let store = Arc::new(InMemoryStore::new());
        let run = create_run(&store).await;
        store
            .update_run_status(run.id, RunStatus::Running)
            .await
            .unwrap();
        store
            .update_run_status(run.id, RunStatus::Completed)
            .await
            .unwrap();

        let (status, _) = send(test_state(store.clone()), "pause", run.id, true).await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        let unchanged = store.get_run(run.id).await.unwrap().unwrap();
        assert_eq!(unchanged.status.state, RunStatus::Completed);
    }

    #[tokio::test]
    async fn pause_run_non_admin_returns_403() {
        let store = Arc::new(InMemoryStore::new());
        let run = create_run(&store).await;

        let (status, _) = send(test_state(store.clone()), "pause", run.id, false).await;

        assert_eq!(status, StatusCode::FORBIDDEN);
        let unchanged = store.get_run(run.id).await.unwrap().unwrap();
        assert_eq!(unchanged.status.state, RunStatus::Pending);
    }

    #[tokio::test]
    async fn pause_unknown_run_returns_404() {
        let store = Arc::new(InMemoryStore::new());

        let (status, _) = send(test_state(store), "pause", Uuid::now_v7(), true).await;

        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn resume_run_returns_pending() {
        let store = Arc::new(InMemoryStore::new());
        let run = create_run(&store).await;
        let state = test_state(store.clone());
        let (status, _) = send(state.clone(), "pause", run.id, true).await;
        assert_eq!(status, StatusCode::OK);

        let (status, body) = send(state, "resume", run.id, true).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["status"], "pending");
        assert!(body["data"].get("resume_status").is_none());
        assert_eq!(body["data"]["resumed_descendants"], json!([]));
        let resumed = store.get_run(run.id).await.unwrap().unwrap();
        assert_eq!(resumed.status.state, RunStatus::Pending);
        assert!(resumed.resume_status.is_none());
    }

    #[tokio::test]
    async fn resume_run_not_paused_returns_400() {
        let store = Arc::new(InMemoryStore::new());
        let run = create_run(&store).await;

        let (status, _) = send(test_state(store), "resume", run.id, true).await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn resume_run_non_admin_returns_403() {
        let store = Arc::new(InMemoryStore::new());
        let run = create_run(&store).await;
        store
            .update_run_status(run.id, RunStatus::Paused)
            .await
            .unwrap();

        let (status, _) = send(test_state(store.clone()), "resume", run.id, false).await;

        assert_eq!(status, StatusCode::FORBIDDEN);
        let unchanged = store.get_run(run.id).await.unwrap().unwrap();
        assert_eq!(unchanged.status.state, RunStatus::Paused);
    }

    #[tokio::test]
    async fn resume_unknown_run_returns_404() {
        let store = Arc::new(InMemoryStore::new());

        let (status, _) = send(test_state(store), "resume", Uuid::now_v7(), true).await;

        assert_eq!(status, StatusCode::NOT_FOUND);
    }
}
