//! `GET /api/v1/internal/runs/:id` — Get run details with steps (raw store entities).

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use serde::Serialize;
use uuid::Uuid;

use ironflow_store::entities::{Run, Step};

use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Internal run detail — raw store entities for the worker.
#[derive(Serialize)]
struct InternalRunDetail {
    run: Run,
    steps: Vec<Step>,
}

/// Get a run by ID with all steps, returning raw store entities.
///
/// Internal routes skip the public DTO conversion so the worker
/// can deserialize the full `FsmState<RunStatus>` and `payload` fields.
pub async fn get_run(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let run = state.get_run_or_404(id).await?;
    let steps = state.store.list_steps(id).await?;

    Ok(ok(InternalRunDetail { run, steps }))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::notify::Event;
    use ironflow_store::entities::{NewStep, StepKind};
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
    async fn get_internal_run_with_steps() {
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

        let step = state
            .store
            .create_step(NewStep {
                run_id: run.id,
                name: "step1".to_string(),
                kind: StepKind::Shell,
                position: 0,
                input: Some(json!({"tool": "test"})),
            })
            .await
            .unwrap();

        let app = create_router(state, RouterConfig::default());

        let req = Request::builder()
            .uri(format!("/api/v1/internal/runs/{}", run.id))
            .header("authorization", "Bearer test-worker-token")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: JsonValue = from_slice(&body).unwrap();
        assert_eq!(json["data"]["run"]["id"], run.id.to_string());
        assert_eq!(json["data"]["steps"].as_array().unwrap().len(), 1);
        assert_eq!(json["data"]["steps"][0]["id"], step.id.to_string());
    }

    #[tokio::test]
    async fn get_internal_run_not_found() {
        let state = test_state();
        let app = create_router(state, RouterConfig::default());

        let fake_id = Uuid::now_v7();
        let req = Request::builder()
            .uri(format!("/api/v1/internal/runs/{}", fake_id))
            .header("authorization", "Bearer test-worker-token")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: JsonValue = from_slice(&body).unwrap();
        assert_eq!(json["error"]["code"], "RUN_NOT_FOUND");
    }
}
