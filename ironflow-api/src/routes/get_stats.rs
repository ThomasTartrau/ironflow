//! `GET /api/v1/stats` — Aggregate statistics across all runs.

use axum::extract::State;
use axum::response::IntoResponse;
use ironflow_auth::extractor::AuthenticatedUser;

use crate::entities::StatsResponse;
use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Get aggregate statistics across all runs.
pub async fn get_stats(
    _user: AuthenticatedUser,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ApiError> {
    let stats = state.store.get_stats().await?;

    let success_rate_percent = if stats.completed_runs + stats.failed_runs > 0 {
        (stats.completed_runs as f64 / (stats.completed_runs + stats.failed_runs) as f64) * 100.0
    } else {
        0.0
    };

    Ok(ok(StatsResponse {
        total_runs: stats.total_runs,
        completed_runs: stats.completed_runs,
        failed_runs: stats.failed_runs,
        cancelled_runs: stats.cancelled_runs,
        active_runs: stats.active_runs,
        success_rate_percent,
        total_cost_usd: stats.total_cost_usd,
        total_duration_ms: stats.total_duration_ms,
    }))
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use http_body_util::BodyExt;
    use ironflow_auth::jwt::AccessToken;
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::engine::Engine;
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::models::{NewRun, RunStatus, TriggerKind};
    use ironflow_store::store::RunStore;
    use serde_json::json;
    use std::sync::Arc;
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::*;

    fn make_auth_header(state: &AppState) -> String {
        let user_id = Uuid::now_v7();
        let token = AccessToken::for_user(user_id, "testuser", false, &state.jwt_config).unwrap();
        format!("Bearer {}", token.0)
    }

    fn test_state(store: Arc<InMemoryStore>) -> AppState {
        let user_store: Arc<dyn ironflow_store::user_store::UserStore> =
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
        AppState {
            store,
            user_store,
            engine,
            jwt_config,
            worker_token: "test-worker-token".to_string(),
        }
    }

    #[tokio::test]
    async fn empty_stats() {
        let store = Arc::new(InMemoryStore::new());
        let state = test_state(store);
        let auth_header = make_auth_header(&state);
        let app = Router::new().route("/", get(get_stats)).with_state(state);

        let req = Request::builder()
            .uri("/")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["data"]["total_runs"], 0);
    }

    #[tokio::test]
    async fn stats_with_mixed_runs() {
        let store = Arc::new(InMemoryStore::new());

        // Pending
        store
            .create_run(NewRun {
                workflow_name: "a".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 0,
            })
            .await
            .unwrap();

        // Completed
        let r2 = store
            .create_run(NewRun {
                workflow_name: "b".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 0,
            })
            .await
            .unwrap();
        store
            .update_run_status(r2.id, RunStatus::Running)
            .await
            .unwrap();
        store
            .update_run_status(r2.id, RunStatus::Completed)
            .await
            .unwrap();

        // Failed
        let r3 = store
            .create_run(NewRun {
                workflow_name: "c".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 0,
            })
            .await
            .unwrap();
        store
            .update_run_status(r3.id, RunStatus::Running)
            .await
            .unwrap();
        store
            .update_run_status(r3.id, RunStatus::Failed)
            .await
            .unwrap();

        let state = test_state(store);
        let auth_header = make_auth_header(&state);
        let app = Router::new().route("/", get(get_stats)).with_state(state);

        let req = Request::builder()
            .uri("/")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["data"]["total_runs"], 3);
        assert_eq!(json["data"]["completed_runs"], 1);
        assert_eq!(json["data"]["failed_runs"], 1);
        assert_eq!(json["data"]["active_runs"], 1);

        let rate = json["data"]["success_rate_percent"].as_f64().unwrap();
        assert!((rate - 50.0).abs() < 0.01);
    }
}
