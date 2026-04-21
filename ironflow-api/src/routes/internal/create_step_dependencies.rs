//! `POST /api/v1/internal/step-dependencies` -- Batch-create step dependency edges.

use axum::Json;
use axum::extract::State;
use axum::response::IntoResponse;

use ironflow_store::entities::NewStepDependency;

use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Create step dependencies in batch (used by the worker during DAG execution).
pub async fn create_step_dependencies(
    State(state): State<AppState>,
    Json(deps): Json<Vec<NewStepDependency>>,
) -> Result<impl IntoResponse, ApiError> {
    use serde_json::json;
    state.store.create_step_dependencies(deps).await?;
    Ok(ok(json!({})))
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::notify::Event;
    use ironflow_store::entities::{NewStep, StepKind};
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::models::{NewRun, TriggerKind};
    use serde_json::{json, to_string};
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
    async fn create_dependencies_happy_path() {
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

        let step_a = state
            .store
            .create_step(NewStep {
                run_id: run.id,
                name: "a".to_string(),
                kind: StepKind::Shell,
                position: 0,
                input: None,
            })
            .await
            .unwrap();

        let step_b = state
            .store
            .create_step(NewStep {
                run_id: run.id,
                name: "b".to_string(),
                kind: StepKind::Shell,
                position: 1,
                input: None,
            })
            .await
            .unwrap();

        let app = create_router(state, RouterConfig::default());

        let body = json!([
            { "step_id": step_b.id, "depends_on": step_a.id }
        ]);

        let req = Request::builder()
            .method("POST")
            .uri("/api/v1/internal/step-dependencies")
            .header("content-type", "application/json")
            .header("authorization", "Bearer test-worker-token")
            .body(Body::from(to_string(&body).unwrap()))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn create_dependencies_empty_array() {
        let state = test_state();
        let app = create_router(state, RouterConfig::default());

        let req = Request::builder()
            .method("POST")
            .uri("/api/v1/internal/step-dependencies")
            .header("content-type", "application/json")
            .header("authorization", "Bearer test-worker-token")
            .body(Body::from("[]"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
