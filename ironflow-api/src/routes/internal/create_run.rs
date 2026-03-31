//! `POST /api/v1/internal/runs` — Create a new run (used by the worker for sub-workflows).

use axum::Json;
use axum::extract::State;
use axum::response::IntoResponse;

use ironflow_store::entities::NewRun;

use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Create a new run (used by the worker for sub-workflow child runs).
///
/// Returns the raw store [`Run`] entity.
pub async fn create_run(
    State(state): State<AppState>,
    Json(req): Json<NewRun>,
) -> Result<impl IntoResponse, ApiError> {
    let run = state.store.create_run(req).await?;
    Ok(ok(run))
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::engine::Engine;
    use ironflow_store::memory::InMemoryStore;
    use serde_json::json;
    use std::sync::Arc;
    use tower::ServiceExt;

    use crate::routes::create_router;
    use crate::state::AppState;

    fn test_state() -> AppState {
        let store = Arc::new(InMemoryStore::new());
        let user_store = Arc::new(InMemoryStore::new());
        let provider = Arc::new(ClaudeCodeProvider::new());
        let engine = Arc::new(Engine::new(store.clone(), provider));
        let jwt_config = Arc::new(ironflow_auth::jwt::JwtConfig {
            secret: "test-secret".to_string(),
            access_token_ttl_secs: 900,
            refresh_token_ttl_secs: 604800,
            cookie_domain: None,
            cookie_secure: false,
        });
        AppState {
            store,
            user_store,
            engine,
            jwt_config,
            worker_token: "test-worker-token".to_string(),
        }
    }

    #[tokio::test]
    async fn create_run_returns_pending() {
        let state = test_state();
        let app = create_router(state, None);

        let req = Request::builder()
            .method("POST")
            .uri("/api/v1/internal/runs")
            .header("authorization", "Bearer test-worker-token")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({
                    "workflow_name": "test-workflow",
                    "trigger": { "kind": "workflow" },
                    "payload": {},
                    "max_retries": 0
                })
                .to_string(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["data"]["workflow_name"], "test-workflow");
        assert_eq!(json["data"]["status"]["state"], "pending");
    }
}
