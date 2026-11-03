//! `POST /api/v1/internal/runs/search` — One page of the runs matching a
//! [`RunFilter`] (raw store entities).

use axum::Json;
use axum::extract::State;
use axum::response::IntoResponse;
use serde::Deserialize;

use ironflow_store::entities::RunFilter;

use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Largest page a worker may ask for, the same ceiling as the public run list.
pub const MAX_SEARCH_PER_PAGE: u32 = 100;

/// Body of [`search_runs`].
#[derive(Debug, Deserialize)]
pub struct SearchRunsRequest {
    /// Filter applied to the runs, as the store takes it.
    #[serde(default)]
    pub filter: RunFilter,
    /// Page number, starting at 1.
    pub page: u32,
    /// Runs per page, from 1 to [`MAX_SEARCH_PER_PAGE`].
    pub per_page: u32,
}

/// List one page of the runs matching a filter, newest first.
///
/// Lets a worker run the engine code that lists runs, such as the token
/// budget of a guard, which reads every run of a tree by its label. The
/// filter travels in a JSON body: its labels do not fit a query string.
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] when `page` is zero or `per_page` is not
/// between 1 and [`MAX_SEARCH_PER_PAGE`].
pub async fn search_runs(
    State(state): State<AppState>,
    Json(request): Json<SearchRunsRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if request.page == 0 {
        return Err(ApiError::BadRequest("page must be at least 1".to_string()));
    }
    if !(1..=MAX_SEARCH_PER_PAGE).contains(&request.per_page) {
        return Err(ApiError::BadRequest(format!(
            "per_page must be between 1 and {MAX_SEARCH_PER_PAGE}"
        )));
    }
    let page = state
        .store
        .list_runs(request.filter, request.page, request.per_page)
        .await?;
    Ok(ok(page))
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
    use ironflow_store::models::{NewRun, Run, TriggerKind};
    use serde_json::{Value, from_slice, json};
    use tokio::sync::broadcast;
    use tower::ServiceExt;

    use super::MAX_SEARCH_PER_PAGE;
    use crate::routes::{RouterConfig, create_router};
    use crate::state::AppState;

    fn test_state() -> AppState {
        let store = Arc::new(InMemoryStore::new());
        let engine = Arc::new(Engine::new(
            store.clone(),
            Arc::new(ClaudeCodeProvider::new()),
        ));
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

    async fn create_run(state: &AppState, labels: HashMap<String, String>) -> Run {
        state
            .store
            .create_run(NewRun {
                created_by: None,
                workflow_name: "test".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 0,
                handler_version: None,
                labels,
                scheduled_at: None,
                idempotency_key: None,
                concurrency_key: None,
                priority: 0,
                concurrency_limits: Vec::new(),
                max_cost_usd: None,
                worker_tags: Vec::new(),
            })
            .await
            .unwrap()
            .into_run()
    }

    async fn search(state: AppState, body: Value, token: &str) -> (StatusCode, Value) {
        let req = Request::builder()
            .method("POST")
            .uri("/api/v1/internal/runs/search")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = create_router(state, RouterConfig::default())
            .oneshot(req)
            .await
            .unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let json = if bytes.is_empty() {
            Value::Null
        } else {
            from_slice(&bytes).unwrap()
        };
        (status, json)
    }

    #[tokio::test]
    async fn filters_runs_by_label() {
        let state = test_state();
        let tree = HashMap::from([("tree".to_string(), "a".to_string())]);
        let in_tree = create_run(&state, tree).await;
        create_run(&state, HashMap::new()).await;

        let (status, json) = search(
            state,
            json!({ "filter": { "labels": { "tree": "a" } }, "page": 1, "per_page": 100 }),
            "test-worker-token",
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"]["total"], 1);
        let items = json["data"]["items"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["id"], in_tree.id.to_string());
    }

    #[tokio::test]
    async fn missing_filter_lists_every_run() {
        let state = test_state();
        create_run(&state, HashMap::new()).await;
        create_run(&state, HashMap::new()).await;

        let (status, json) = search(
            state,
            json!({ "page": 1, "per_page": 1 }),
            "test-worker-token",
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"]["total"], 2);
        assert_eq!(json["data"]["items"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn page_zero_is_rejected() {
        let (status, _) = search(
            test_state(),
            json!({ "page": 0, "per_page": 10 }),
            "test-worker-token",
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn per_page_out_of_range_is_rejected() {
        for per_page in [0, MAX_SEARCH_PER_PAGE + 1] {
            let (status, _) = search(
                test_state(),
                json!({ "page": 1, "per_page": per_page }),
                "test-worker-token",
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "per_page {per_page}");
        }
    }

    #[tokio::test]
    async fn wrong_worker_token_is_rejected() {
        let (status, _) = search(
            test_state(),
            json!({ "page": 1, "per_page": 10 }),
            "not-the-token",
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
}
