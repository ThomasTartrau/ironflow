//! `GET /api/v1/internal/runs/pending-count` — Number of runs waiting for a worker.

use axum::extract::State;
use axum::response::IntoResponse;
use serde::Serialize;

use ironflow_store::entities::{RunFilter, RunStatus};

use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Queue depth as seen by the store.
#[derive(Serialize)]
struct PendingCount {
    pending_runs: u64,
}

/// Count the runs in the `Pending` state.
///
/// A worker has no store of its own: it reads this count to publish
/// `ironflow_worker_queue_depth`.
pub async fn count_pending_runs(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ApiError> {
    let stats = state
        .store
        .get_stats(RunFilter {
            status: Some(RunStatus::Pending),
            ..RunFilter::default()
        })
        .await?;

    Ok(ok(PendingCount {
        pending_runs: stats.total_runs,
    }))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use ironflow_auth::jwt::JwtConfig;
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::notify::Event;
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::models::{NewRun, RunStatus, TriggerKind};
    use serde_json::{Value as JsonValue, from_slice, json};
    use tokio::sync::broadcast;
    use tower::ServiceExt;

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
            max_cost_usd: None,
        }
    }

    fn request(authorization: Option<&str>) -> Request<Body> {
        let builder = Request::builder().uri("/api/v1/internal/runs/pending-count");
        let builder = match authorization {
            Some(value) => builder.header("authorization", value),
            None => builder,
        };
        builder.body(Body::empty()).unwrap()
    }

    #[tokio::test]
    async fn count_pending_runs_counts_only_pending_runs() {
        let state = test_state();
        for _ in 0..2 {
            state.store.create_run(new_run()).await.unwrap();
        }
        let running = state.store.create_run(new_run()).await.unwrap().into_run();
        state
            .store
            .update_run_status(running.id, RunStatus::Running)
            .await
            .unwrap();

        let app = create_router(state, RouterConfig::default());
        let resp = app
            .oneshot(request(Some("Bearer test-worker-token")))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: JsonValue = from_slice(&body).unwrap();
        assert_eq!(json["data"]["pending_runs"], 2);
    }

    #[tokio::test]
    async fn count_pending_runs_is_zero_on_empty_store() {
        let app = create_router(test_state(), RouterConfig::default());
        let resp = app
            .oneshot(request(Some("Bearer test-worker-token")))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: JsonValue = from_slice(&body).unwrap();
        assert_eq!(json["data"]["pending_runs"], 0);
    }

    #[tokio::test]
    async fn count_pending_runs_rejects_missing_or_wrong_worker_token() {
        for authorization in [None, Some("Bearer wrong-token")] {
            let app = create_router(test_state(), RouterConfig::default());
            let resp = app.oneshot(request(authorization)).await.unwrap();
            assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "{authorization:?}");

            let body = resp.into_body().collect().await.unwrap().to_bytes();
            let json: JsonValue = from_slice(&body).unwrap();
            assert_eq!(json["error"]["code"], "INVALID_WORKER_TOKEN");
        }
    }
}
