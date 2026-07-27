//! `GET /api/v1/internal/runs/next` — Pick the next pending run for execution.

use axum::extract::State;
use axum::response::IntoResponse;
use chrono::Utc;
use rust_decimal::Decimal;

use ironflow_engine::notify::Event;
use ironflow_store::models::RunStatus;

use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Atomically pick the next pending run and transition it to Running.
///
/// Returns the raw store [`Run`] entity or null if no pending runs are available.
/// Internal routes return store entities (not public DTOs) because the worker
/// needs the full `FsmState<RunStatus>` and `payload` fields.
pub async fn pick_next_run(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    let run = state.store.pick_next_pending().await?;

    if let Some(ref picked) = run {
        state
            .engine
            .event_publisher()
            .publish(Event::RunStatusChanged {
                run_id: picked.id,
                workflow_name: picked.workflow_name.clone(),
                from: RunStatus::Pending,
                to: RunStatus::Running,
                error: None,
                cost_usd: Decimal::ZERO,
                duration_ms: 0,
                at: Utc::now(),
            });
    }

    Ok(ok(run))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::notify::Event;
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::models::{NewRun, TriggerKind};
    use serde_json::{Value as JsonValue, from_slice, json};
    use std::sync::Arc;
    use tokio::sync::broadcast;
    use tower::ServiceExt;

    use crate::routes::{RouterConfig, create_router};
    use crate::state::AppState;

    fn test_state() -> AppState {
        let store = Arc::new(InMemoryStore::new());
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
    async fn pick_next_returns_pending_run() {
        let state = test_state();
        let run = state
            .store
            .create_run(NewRun {
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 0,
                handler_version: None,
                labels: HashMap::new(),
                scheduled_at: None,
                idempotency_key: None,
            })
            .await
            .unwrap()
            .into_run();

        let app = create_router(state, RouterConfig::default());

        let req = Request::builder()
            .uri("/api/v1/internal/runs/next")
            .header("authorization", "Bearer test-worker-token")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        assert_eq!(json_val["data"]["id"], run.id.to_string());
    }

    #[tokio::test]
    async fn pick_next_empty_returns_null() {
        let state = test_state();
        let app = create_router(state, RouterConfig::default());

        let req = Request::builder()
            .uri("/api/v1/internal/runs/next")
            .header("authorization", "Bearer test-worker-token")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        assert!(json_val["data"].is_null());
    }

    #[tokio::test]
    async fn pick_next_transitions_to_running() {
        let state = test_state();
        let _run = state
            .store
            .create_run(NewRun {
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 0,
                handler_version: None,
                labels: HashMap::new(),
                scheduled_at: None,
                idempotency_key: None,
            })
            .await
            .unwrap()
            .into_run();

        let app = create_router(state.clone(), RouterConfig::default());

        let req = Request::builder()
            .uri("/api/v1/internal/runs/next")
            .header("authorization", "Bearer test-worker-token")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        assert_eq!(json_val["data"]["status"]["state"], "running");
    }
}
