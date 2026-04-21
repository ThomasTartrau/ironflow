//! `POST /api/v1/internal/steps` — Create a new step for a run.

use axum::Json;
use axum::extract::State;
use axum::response::IntoResponse;

use ironflow_store::entities::NewStep;

use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Create a new step (used by the worker during workflow execution).
///
/// Returns the raw store [`Step`] entity — internal routes skip the public DTO
/// so the worker can deserialize the full entity.
pub async fn create_step(
    State(state): State<AppState>,
    Json(req): Json<NewStep>,
) -> Result<impl IntoResponse, ApiError> {
    let step = state.store.create_step(req).await?;
    Ok(ok(step))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::notify::Event;
    use ironflow_store::entities::StepKind;
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::models::{NewRun, TriggerKind};
    use serde_json::{Value as JsonValue, json, to_string};
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
    async fn create_step_success() {
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

        let new_step = NewStep {
            run_id: run.id,
            name: "step1".to_string(),
            kind: StepKind::Shell,
            position: 0,
            input: Some(json!({"tool": "test"})),
        };

        let req = Request::builder()
            .method("POST")
            .uri("/api/v1/internal/steps")
            .header("authorization", "Bearer test-worker-token")
            .header("content-type", "application/json")
            .body(Body::from(to_string(&new_step).unwrap()))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = serde_json::from_slice(&body).unwrap();
        assert_eq!(json_val["data"]["run_id"], run.id.to_string());
    }

    #[tokio::test]
    async fn create_step_run_not_found() {
        let state = test_state();
        let app = create_router(state, RouterConfig::default());

        let fake_run_id = Uuid::now_v7();
        let new_step = NewStep {
            run_id: fake_run_id,
            name: "step1".to_string(),
            kind: StepKind::Shell,
            position: 0,
            input: Some(json!({"tool": "test"})),
        };

        let req = Request::builder()
            .method("POST")
            .uri("/api/v1/internal/steps")
            .header("authorization", "Bearer test-worker-token")
            .header("content-type", "application/json")
            .body(Body::from(to_string(&new_step).unwrap()))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
