//! `POST /api/v1/internal/runs/:id/lease` — Refresh a worker lease on a run.

use axum::Json;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use ironflow_store::error::StoreError;
use uuid::Uuid;

use crate::entities::{RenewLeaseRequest, RenewLeaseResponse, validate_lease_ttl};
use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Extend the lease the calling worker holds on a run.
///
/// The worker calls this while it executes a run. Once the worker dies, the
/// lease expires and the reaper requeues the run.
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] if `worker_id` is blank or `lease_ttl_secs`
/// is out of range.
/// Returns 404 if the run does not exist.
/// Returns 409 (`LEASE_LOST`) if the run is no longer `Running` or if another
/// worker owns the lease — the caller must stop executing the run.
pub async fn renew_lease(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<RenewLeaseRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let lease = validate_lease_ttl(Some(body.worker_id), body.lease_ttl_secs)?
        .ok_or_else(|| ApiError::BadRequest("worker_id is required".into()))?;

    let lease_expires_at = state
        .store
        .renew_lease(id, lease)
        .await
        .map_err(|err| match err {
            StoreError::RunNotFound(id) => ApiError::RunNotFound(id),
            other => ApiError::Store(other),
        })?;

    Ok(ok(RenewLeaseResponse { lease_expires_at }))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::time::Duration;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use ironflow_auth::jwt::JwtConfig;
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::notify::Event;
    use ironflow_store::entities::LeaseRequest;
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::models::{NewRun, RunStatus, TriggerKind};
    use serde_json::{Value as JsonValue, from_slice, json};
    use std::sync::Arc;
    use tokio::sync::broadcast;
    use tower::ServiceExt;
    use uuid::Uuid;

    use crate::routes::{RouterConfig, create_router};
    use crate::state::AppState;

    fn test_state() -> AppState {
        let store = Arc::new(InMemoryStore::new());
        let provider = Arc::new(ClaudeCodeProvider::new());
        let engine = Arc::new(Engine::new(store.clone(), provider));
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
            engine,
            jwt_config,
            "test-worker-token".to_string(),
            event_sender,
        )
    }

    fn new_run() -> NewRun {
        NewRun {
            workflow_name: "test".to_string(),
            trigger: TriggerKind::Manual,
            payload: json!({}),
            max_retries: 3,
            handler_version: None,
            labels: HashMap::new(),
            scheduled_at: None,
            created_by: None,
            idempotency_key: None,
            max_cost_usd: None,
        }
    }

    fn lease_request(id: Uuid, body: JsonValue) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(format!("/api/v1/internal/runs/{id}/lease"))
            .header("authorization", "Bearer test-worker-token")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    async fn picked_run(state: &AppState, worker_id: &str) -> Uuid {
        state.store.create_run(new_run()).await.unwrap();
        state
            .store
            .pick_next_pending(Some(LeaseRequest {
                worker_id: worker_id.to_string(),
                ttl: Duration::from_secs(90),
            }))
            .await
            .unwrap()
            .unwrap()
            .id
    }

    #[tokio::test]
    async fn renew_returns_new_expiry_for_owner() {
        let state = test_state();
        let run_id = picked_run(&state, "worker-1").await;
        let app = create_router(state, RouterConfig::default());

        let resp = app
            .oneshot(lease_request(
                run_id,
                json!({"worker_id": "worker-1", "lease_ttl_secs": 90}),
            ))
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        assert!(json_val["data"]["lease_expires_at"].is_string());
    }

    #[tokio::test]
    async fn renew_by_other_worker_conflicts() {
        let state = test_state();
        let run_id = picked_run(&state, "worker-1").await;
        let app = create_router(state, RouterConfig::default());

        let resp = app
            .oneshot(lease_request(run_id, json!({"worker_id": "worker-2"})))
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::CONFLICT);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        assert_eq!(json_val["error"]["code"], "LEASE_LOST");
    }

    #[tokio::test]
    async fn renew_on_cancelled_run_conflicts() {
        let state = test_state();
        let run_id = picked_run(&state, "worker-1").await;
        state
            .store
            .update_run_status(run_id, RunStatus::Cancelled)
            .await
            .unwrap();
        let app = create_router(state, RouterConfig::default());

        let resp = app
            .oneshot(lease_request(run_id, json!({"worker_id": "worker-1"})))
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn renew_unknown_run_returns_404() {
        let state = test_state();
        let app = create_router(state, RouterConfig::default());

        let resp = app
            .oneshot(lease_request(
                Uuid::now_v7(),
                json!({"worker_id": "worker-1"}),
            ))
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn renew_with_invalid_ttl_returns_400() {
        let state = test_state();
        let run_id = picked_run(&state, "worker-1").await;
        let app = create_router(state, RouterConfig::default());

        let resp = app
            .oneshot(lease_request(
                run_id,
                json!({"worker_id": "worker-1", "lease_ttl_secs": 0}),
            ))
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn renew_without_worker_token_is_unauthorized() {
        let state = test_state();
        let run_id = picked_run(&state, "worker-1").await;
        let app = create_router(state, RouterConfig::default());

        let req = Request::builder()
            .method("POST")
            .uri(format!("/api/v1/internal/runs/{run_id}/lease"))
            .header("content-type", "application/json")
            .body(Body::from(json!({"worker_id": "worker-1"}).to_string()))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}
