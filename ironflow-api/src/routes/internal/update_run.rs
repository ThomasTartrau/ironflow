//! `PUT /api/v1/internal/runs/:id` — Update run fields.

use axum::Json;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use uuid::Uuid;

use serde_json::json;

use ironflow_store::entities::RunUpdate;

use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Update run fields (cost, duration, error, timestamps).
pub async fn update_run(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(update): Json<RunUpdate>,
) -> Result<impl IntoResponse, ApiError> {
    state.store.update_run(id, update).await?;
    Ok(ok(json!({ "updated": true })))
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
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::models::{NewRun, TriggerKind};
    use rust_decimal::Decimal;
    use serde_json::{Value as JsonValue, from_slice, json, to_string};
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
    async fn update_run_cost_and_duration() {
        let state = test_state();
        let run = state
            .store
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
                max_cost_usd: None,
            })
            .await
            .unwrap()
            .into_run();

        let app = create_router(state.clone(), RouterConfig::default());

        let update = RunUpdate {
            status: None,
            error: None,
            cost_usd: Some(Decimal::from_str_exact("1.50").unwrap()),
            duration_ms: Some(5000),
            started_at: None,
            completed_at: None,
            increment_retry: false,
            scheduled_at: None,
        };

        let req = Request::builder()
            .method("PUT")
            .uri(format!("/api/v1/internal/runs/{}", run.id))
            .header("authorization", "Bearer test-worker-token")
            .header("content-type", "application/json")
            .body(Body::from(to_string(&update).unwrap()))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        assert_eq!(json_val["data"]["updated"], true);

        let updated = state.get_run_or_404(run.id).await.unwrap();
        assert_eq!(updated.cost_usd, Decimal::from_str_exact("1.50").unwrap());
        assert_eq!(updated.duration_ms, 5000);
    }

    #[tokio::test]
    async fn update_run_not_found() {
        let state = test_state();
        let app = create_router(state, RouterConfig::default());

        let fake_id = Uuid::now_v7();
        let update = RunUpdate {
            status: None,
            error: None,
            cost_usd: Some(Decimal::from_str_exact("1.00").unwrap()),
            duration_ms: None,
            started_at: None,
            completed_at: None,
            increment_retry: false,
            scheduled_at: None,
        };

        let req = Request::builder()
            .method("PUT")
            .uri(format!("/api/v1/internal/runs/{}", fake_id))
            .header("authorization", "Bearer test-worker-token")
            .header("content-type", "application/json")
            .body(Body::from(to_string(&update).unwrap()))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
