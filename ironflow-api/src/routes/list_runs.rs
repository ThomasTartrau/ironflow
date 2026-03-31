//! `GET /api/v1/runs` — List runs with filtering and pagination.

use axum::extract::{Query, State};
use axum::response::IntoResponse;
use ironflow_auth::extractor::AuthenticatedUser;
use ironflow_store::models::RunFilter;

use crate::entities::{ListRunsQuery, RunResponse};
use crate::error::ApiError;
use crate::response::ok_paged;
use crate::state::AppState;

/// List runs with optional filtering and pagination.
///
/// # Query Parameters
///
/// - `workflow` — Filter by workflow name (optional)
/// - `status` — Filter by run status (optional)
/// - `page` — Page number, 1-based (default: 1)
/// - `per_page` — Items per page (default: 20, max: 100)
pub async fn list_runs(
    _user: AuthenticatedUser,
    State(state): State<AppState>,
    Query(params): Query<ListRunsQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let page = params.page.unwrap_or(1);
    let per_page = params.per_page.unwrap_or(20).min(100);

    let filter = RunFilter {
        workflow_name: params.workflow,
        status: params.status,
        created_after: None,
        created_before: None,
    };

    let page_result = state.store.list_runs(filter, page, per_page).await?;
    let runs: Vec<RunResponse> = page_result
        .items
        .into_iter()
        .map(RunResponse::from)
        .collect();

    Ok(ok_paged(runs, page, per_page, page_result.total))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use http_body_util::BodyExt;
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::engine::Engine;
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::models::{NewRun, TriggerKind};
    use serde_json::json;
    use std::sync::Arc;
    use tower::ServiceExt;

    fn test_state() -> AppState {
        let store = Arc::new(InMemoryStore::new());
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

    fn make_auth_header(state: &AppState) -> String {
        use ironflow_auth::jwt::AccessToken;
        use uuid::Uuid;

        let user_id = Uuid::now_v7();
        let token = AccessToken::for_user(user_id, "testuser", false, &state.jwt_config).unwrap();
        format!("Bearer {}", token.0)
    }

    #[tokio::test]
    async fn empty_list() {
        let state = test_state();
        let auth_header = make_auth_header(&state);
        let app = Router::new().route("/", get(list_runs)).with_state(state);

        let req = Request::builder()
            .uri("/?page=1&per_page=20")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["data"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn with_workflow_filter() {
        let state = test_state();
        let auth_header = make_auth_header(&state);
        state
            .store
            .create_run(NewRun {
                workflow_name: "deploy".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 0,
            })
            .await
            .unwrap();
        state
            .store
            .create_run(NewRun {
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 0,
            })
            .await
            .unwrap();

        let app = Router::new().route("/", get(list_runs)).with_state(state);

        let req = Request::builder()
            .uri("/?workflow=deploy")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["data"].as_array().unwrap().len(), 1);
        assert_eq!(json["data"][0]["workflow_name"], "deploy");
    }

    #[tokio::test]
    async fn with_status_filter() {
        let state = test_state();
        let auth_header = make_auth_header(&state);

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
            .update_run_status(run.id, ironflow_store::models::RunStatus::Running)
            .await
            .unwrap();

        // Second run stays Pending
        state
            .store
            .create_run(NewRun {
                workflow_name: "other".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 0,
            })
            .await
            .unwrap();

        let app = Router::new().route("/", get(list_runs)).with_state(state);

        let req = Request::builder()
            .uri("/?status=running")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["data"].as_array().unwrap().len(), 1);
        assert_eq!(json["data"][0]["status"], "running");
    }

    #[tokio::test]
    async fn pagination_meta_returned() {
        let state = test_state();
        let auth_header = make_auth_header(&state);

        for i in 0..5 {
            state
                .store
                .create_run(NewRun {
                    workflow_name: format!("wf-{i}"),
                    trigger: TriggerKind::Manual,
                    payload: json!({}),
                    max_retries: 0,
                })
                .await
                .unwrap();
        }

        let app = Router::new().route("/", get(list_runs)).with_state(state);

        let req = Request::builder()
            .uri("/?page=1&per_page=2")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["data"].as_array().unwrap().len(), 2);
        assert_eq!(json["meta"]["page"], 1);
        assert_eq!(json["meta"]["per_page"], 2);
        assert_eq!(json["meta"]["total"], 5);
    }

    #[tokio::test]
    async fn per_page_capped_at_100() {
        let state = test_state();
        let auth_header = make_auth_header(&state);
        let app = Router::new().route("/", get(list_runs)).with_state(state);

        let req = Request::builder()
            .uri("/?per_page=500")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        // per_page should be capped to 100
        assert_eq!(json["meta"]["per_page"], 100);
    }
}
