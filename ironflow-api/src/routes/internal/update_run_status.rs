//! `PUT /api/v1/internal/runs/:id/status` — Update a run's status.

use axum::Json;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use chrono::Utc;
use rust_decimal::Decimal;
use serde::Deserialize;
use uuid::Uuid;

use serde_json::json;

use ironflow_store::entities::{RunStatus, RunUpdate};

use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Request body for updating run status.
#[derive(Debug, Deserialize, serde::Serialize)]
pub struct UpdateRunStatusRequest {
    /// New status for the run.
    pub status: RunStatus,
    /// Optional error message (for failed runs).
    pub error: Option<String>,
    /// Optional cost in USD.
    pub cost_usd: Option<Decimal>,
    /// Optional duration in milliseconds.
    pub duration_ms: Option<u64>,
}

/// Update a run's status (used by the worker to report completion/failure).
///
/// Combines status transition and metadata update into a single atomic
/// `update_run` call to avoid inconsistent state.
pub async fn update_run_status(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateRunStatusRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let now = Utc::now();
    let started_at = if req.status == RunStatus::Running {
        Some(now)
    } else {
        None
    };
    let completed_at = if req.status.is_terminal() {
        Some(now)
    } else {
        None
    };

    let update = RunUpdate {
        status: Some(req.status),
        error: req.error,
        cost_usd: req.cost_usd,
        duration_ms: req.duration_ms,
        started_at,
        completed_at,
        increment_retry: false,
    };
    state.store.update_run(id, update).await?;

    Ok(ok(json!({ "updated": true })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use serde_json::to_string;

    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::notify::Event;
    use ironflow_store::api_key_store::ApiKeyStore;
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::models::{NewRun, TriggerKind};
    use ironflow_store::user_store::UserStore;
    use serde_json::json;
    use std::sync::Arc;
    use tokio::sync::broadcast;
    use tower::ServiceExt;

    use crate::routes::{RouterConfig, create_router};
    use crate::state::AppState;

    fn test_state() -> AppState {
        let store = Arc::new(InMemoryStore::new());
        let user_store: Arc<dyn UserStore> = Arc::new(InMemoryStore::new());
        let api_key_store: Arc<dyn ApiKeyStore> = Arc::new(InMemoryStore::new());
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
            user_store,
            api_key_store,
            engine,
            jwt_config,
            "test-worker-token".to_string(),
            event_sender,
        )
    }

    #[tokio::test]
    async fn update_status_to_running() {
        let state = test_state();
        let run = state
            .store
            .create_run(NewRun {
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 0,
            })
            .await
            .unwrap();

        let app = create_router(state.clone(), RouterConfig::default());

        let req_body = UpdateRunStatusRequest {
            status: RunStatus::Running,
            error: None,
            cost_usd: None,
            duration_ms: None,
        };

        let req = Request::builder()
            .method("PUT")
            .uri(format!("/api/v1/internal/runs/{}/status", run.id))
            .header("authorization", "Bearer test-worker-token")
            .header("content-type", "application/json")
            .body(Body::from(to_string(&req_body).unwrap()))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let updated = state.get_run_or_404(run.id).await.unwrap();
        assert_eq!(updated.status.state, RunStatus::Running);
    }

    #[tokio::test]
    async fn update_status_to_completed() {
        let state = test_state();
        let run = state
            .store
            .create_run(NewRun {
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 0,
            })
            .await
            .unwrap();

        state
            .store
            .update_run(
                run.id,
                RunUpdate {
                    status: Some(RunStatus::Running),
                    error: None,
                    cost_usd: None,
                    duration_ms: None,
                    started_at: Some(Utc::now()),
                    completed_at: None,
                    increment_retry: false,
                },
            )
            .await
            .unwrap();

        let app = create_router(state.clone(), RouterConfig::default());

        let req_body = UpdateRunStatusRequest {
            status: RunStatus::Completed,
            error: None,
            cost_usd: Some(Decimal::from_str_exact("2.00").unwrap()),
            duration_ms: Some(3000),
        };

        let req = Request::builder()
            .method("PUT")
            .uri(format!("/api/v1/internal/runs/{}/status", run.id))
            .header("authorization", "Bearer test-worker-token")
            .header("content-type", "application/json")
            .body(Body::from(to_string(&req_body).unwrap()))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let updated = state.get_run_or_404(run.id).await.unwrap();
        assert_eq!(updated.status.state, RunStatus::Completed);
        assert_eq!(updated.cost_usd, Decimal::from_str_exact("2.00").unwrap());
        assert_eq!(updated.duration_ms, 3000);
    }

    #[tokio::test]
    async fn update_status_not_found() {
        let state = test_state();
        let app = create_router(state, RouterConfig::default());

        let fake_id = Uuid::now_v7();
        let req_body = UpdateRunStatusRequest {
            status: RunStatus::Running,
            error: None,
            cost_usd: None,
            duration_ms: None,
        };

        let req = Request::builder()
            .method("PUT")
            .uri(format!("/api/v1/internal/runs/{}/status", fake_id))
            .header("authorization", "Bearer test-worker-token")
            .header("content-type", "application/json")
            .body(Body::from(to_string(&req_body).unwrap()))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
