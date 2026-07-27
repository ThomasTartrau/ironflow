//! `POST /api/v1/runs/:id/cancel` — Cancel a pending or running run.

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use chrono::Utc;
use ironflow_auth::extractor::Authenticated;
use ironflow_engine::notify::Event;
use ironflow_store::models::RunStatus;
use uuid::Uuid;

use crate::entities::RunResponse;
use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Cancel a pending or running run.
///
/// Transitions the run to `Cancelled` status. Returns 400 if the run
/// is already in a terminal state.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        post,
        path = "/api/v1/runs/{id}/cancel",
        tags = ["runs"],
        params(("id" = Uuid, Path, description = "Run ID")),
        responses(
            (status = 200, description = "Run cancelled successfully", body = RunResponse),
            (status = 400, description = "Run cannot be cancelled"),
            (status = 401, description = "Unauthorized"),
            (status = 403, description = "Forbidden"),
            (status = 404, description = "Run not found")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn cancel_run(
    auth: Authenticated,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    if !auth.is_admin() {
        return Err(ApiError::Forbidden);
    }

    let run = state.get_run_or_404(id).await?;

    if !run.status.state.can_transition_to(&RunStatus::Cancelled) {
        return Err(ApiError::BadRequest(format!(
            "cannot cancel run in {} state",
            run.status.state
        )));
    }

    state
        .store
        .update_run_status(id, RunStatus::Cancelled)
        .await?;

    let cancelled = state.get_run_or_404(id).await?;

    state
        .engine
        .event_publisher()
        .publish(Event::RunStatusChanged {
            run_id: id,
            workflow_name: cancelled.workflow_name.clone(),
            from: run.status.state,
            to: RunStatus::Cancelled,
            error: None,
            cost_usd: cancelled.cost_usd,
            duration_ms: cancelled.duration_ms,
            at: Utc::now(),
        });

    Ok(ok(RunResponse::from(cancelled)))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode as HttpStatusCode};
    use axum::routing::post;
    use http_body_util::BodyExt;
    use ironflow_auth::jwt::AccessToken;
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::notify::Event;
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::models::{NewRun, RunStatus, TriggerKind};
    use ironflow_store::store::RunStore;
    use serde_json::{Value as JsonValue, from_slice, json};
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

    fn test_state(store: Arc<InMemoryStore>) -> AppState {
        Arc::new(InMemoryStore::new());
        let provider = Arc::new(ClaudeCodeProvider::new());
        let engine = Arc::new(Engine::new(store.clone(), provider));
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
            engine,
            jwt_config,
            "test-worker-token".to_string(),
            event_sender,
        )
    }

    #[tokio::test]
    async fn cancel_pending_run() {
        let store = Arc::new(InMemoryStore::new());
        let run = store
            .create_run(NewRun {
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 0,
                handler_version: None,
                labels: HashMap::new(),
                scheduled_at: None,
                max_cost_usd: None,
            })
            .await
            .unwrap();

        let state = test_state(store.clone());
        let auth_header = make_auth_header(&state);
        let app = Router::new()
            .route("/{id}/cancel", post(cancel_run))
            .with_state(state);

        let req = Request::builder()
            .method("POST")
            .uri(format!("/{}/cancel", run.id))
            .header("content-type", "application/json")
            .header("authorization", auth_header)
            .body(Body::from("{}"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), HttpStatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        assert_eq!(json_val["data"]["status"], "cancelled");

        let cancelled = store.get_run(run.id).await.unwrap().unwrap();
        assert_eq!(cancelled.status.state, RunStatus::Cancelled);
    }

    #[tokio::test]
    async fn cancel_completed_run_returns_400() {
        let store = Arc::new(InMemoryStore::new());
        let run = store
            .create_run(NewRun {
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 0,
                handler_version: None,
                labels: HashMap::new(),
                scheduled_at: None,
                max_cost_usd: None,
            })
            .await
            .unwrap();

        store
            .update_run_status(run.id, RunStatus::Running)
            .await
            .unwrap();
        store
            .update_run_status(run.id, RunStatus::Completed)
            .await
            .unwrap();

        let state = test_state(store);
        let auth_header = make_auth_header(&state);
        let app = Router::new()
            .route("/{id}/cancel", post(cancel_run))
            .with_state(state);

        let req = Request::builder()
            .method("POST")
            .uri(format!("/{}/cancel", run.id))
            .header("content-type", "application/json")
            .header("authorization", auth_header)
            .body(Body::from("{}"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), HttpStatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn cancel_running_run() {
        let store = Arc::new(InMemoryStore::new());
        let run = store
            .create_run(NewRun {
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 0,
                handler_version: None,
                labels: HashMap::new(),
                scheduled_at: None,
                max_cost_usd: None,
            })
            .await
            .unwrap();

        store
            .update_run_status(run.id, RunStatus::Running)
            .await
            .unwrap();

        let state = test_state(store);
        let auth_header = make_auth_header(&state);
        let app = Router::new()
            .route("/{id}/cancel", post(cancel_run))
            .with_state(state);

        let req = Request::builder()
            .method("POST")
            .uri(format!("/{}/cancel", run.id))
            .header("content-type", "application/json")
            .header("authorization", auth_header)
            .body(Body::from("{}"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), HttpStatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        assert_eq!(json_val["data"]["status"], "cancelled");
    }

    #[tokio::test]
    async fn cancel_failed_run_returns_400() {
        let store = Arc::new(InMemoryStore::new());
        let run = store
            .create_run(NewRun {
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 0,
                handler_version: None,
                labels: HashMap::new(),
                scheduled_at: None,
                max_cost_usd: None,
            })
            .await
            .unwrap();

        store
            .update_run_status(run.id, RunStatus::Running)
            .await
            .unwrap();
        store
            .update_run_status(run.id, RunStatus::Failed)
            .await
            .unwrap();

        let state = test_state(store);
        let auth_header = make_auth_header(&state);
        let app = Router::new()
            .route("/{id}/cancel", post(cancel_run))
            .with_state(state);

        let req = Request::builder()
            .method("POST")
            .uri(format!("/{}/cancel", run.id))
            .header("content-type", "application/json")
            .header("authorization", auth_header)
            .body(Body::from("{}"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), HttpStatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn cancel_nonexistent_run_returns_404() {
        let store = Arc::new(InMemoryStore::new());
        let state = test_state(store);
        let auth_header = make_auth_header(&state);
        let app = Router::new()
            .route("/{id}/cancel", post(cancel_run))
            .with_state(state);

        let req = Request::builder()
            .method("POST")
            .uri(format!("/{}/cancel", Uuid::now_v7()))
            .header("content-type", "application/json")
            .header("authorization", auth_header)
            .body(Body::from("{}"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), HttpStatusCode::NOT_FOUND);
    }
}
