//! `GET /api/v1/workflows` — List registered workflows.

use axum::extract::{Query, State};
use axum::response::IntoResponse;
use ironflow_auth::extractor::Authenticated;
use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Query parameters for listing workflows.
#[cfg_attr(feature = "openapi", derive(utoipa::IntoParams, utoipa::ToSchema))]
#[derive(Debug, Deserialize)]
pub struct ListWorkflowsQuery {
    /// Optional case-insensitive partial match on workflow name.
    pub name: Option<String>,
    /// Optional case-insensitive partial match on the category path
    /// (e.g. `etl` matches `Data/ETL` and `data/etl/nightly`).
    ///
    /// Pass `__uncategorized__` to list only workflows without any
    /// category.
    pub category: Option<String>,
}

/// Summary entry returned by `GET /api/v1/workflows`.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Serialize, Deserialize)]
pub struct WorkflowSummary {
    /// Workflow name (unique identifier).
    pub name: String,
    /// Optional `/`-separated category path.
    pub category: Option<String>,
    /// Current handler version.
    pub version: Option<String>,
}

/// Sentinel value for the `category` query parameter that selects only
/// uncategorized workflows.
pub const UNCATEGORIZED_FILTER: &str = "__uncategorized__";

/// List registered workflows, optionally filtered by name and category.
///
/// # Query Parameters
///
/// - `name` — Case-insensitive partial match on workflow name (optional)
/// - `category` — Case-insensitive partial match on category path, or
///   [`UNCATEGORIZED_FILTER`] to filter only uncategorized workflows
///   (optional)
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/workflows",
        tags = ["workflows"],
        params(ListWorkflowsQuery),
        responses(
            (status = 200, description = "List of workflow summaries", body = [WorkflowSummary]),
            (status = 401, description = "Unauthorized")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn list_workflows(
    _auth: Authenticated,
    State(state): State<AppState>,
    Query(params): Query<ListWorkflowsQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let mut summaries: Vec<WorkflowSummary> = state
        .engine
        .handler_names()
        .into_iter()
        .map(|name| {
            let info = state.engine.handler_info(name);
            let category = info.as_ref().and_then(|i| i.category.clone());
            let version = info.and_then(|i| i.version);
            WorkflowSummary {
                name: name.to_string(),
                category,
                version,
            }
        })
        .collect();

    if let Some(ref filter) = params.name {
        let lower = filter.to_lowercase();
        summaries.retain(|s| s.name.to_lowercase().contains(&lower));
    }

    if let Some(ref cat_filter) = params.category {
        if cat_filter == UNCATEGORIZED_FILTER {
            summaries.retain(|s| s.category.is_none());
        } else {
            let needle = cat_filter.to_lowercase();
            summaries.retain(|s| {
                s.category
                    .as_deref()
                    .is_some_and(|c| c.to_lowercase().contains(&needle))
            });
        }
    }

    summaries.sort_by(|a, b| {
        a.category
            .as_deref()
            .unwrap_or("")
            .cmp(b.category.as_deref().unwrap_or(""))
            .then_with(|| a.name.cmp(&b.name))
    });

    Ok(ok(summaries))
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
    use ironflow_engine::context::WorkflowContext;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
    use ironflow_engine::notify::Event;
    use ironflow_store::memory::InMemoryStore;
    use serde_json::{Value as JsonValue, from_slice, from_value};
    use std::sync::Arc;
    use tokio::sync::broadcast;
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::*;

    fn make_auth_header(state: &AppState) -> String {
        let user_id = Uuid::now_v7();
        let token = AccessToken::for_user(user_id, "testuser", false, &state.jwt_config).unwrap();
        format!("Bearer {}", token.0)
    }

    struct TestWorkflow;

    impl WorkflowHandler for TestWorkflow {
        fn name(&self) -> &str {
            "test-workflow"
        }

        fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
            Box::pin(async move { Ok(()) })
        }
    }

    struct AnotherWorkflow;

    impl WorkflowHandler for AnotherWorkflow {
        fn name(&self) -> &str {
            "another-workflow"
        }

        fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
            Box::pin(async move { Ok(()) })
        }
    }

    struct EtlNightlyWorkflow;

    impl WorkflowHandler for EtlNightlyWorkflow {
        fn name(&self) -> &str {
            "etl-nightly"
        }
        fn category(&self) -> Option<&str> {
            Some("data/etl")
        }
        fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
            Box::pin(async move { Ok(()) })
        }
    }

    struct ReportsDailyWorkflow;

    impl WorkflowHandler for ReportsDailyWorkflow {
        fn name(&self) -> &str {
            "reports-daily"
        }
        fn category(&self) -> Option<&str> {
            Some("data/reports")
        }
        fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
            Box::pin(async move { Ok(()) })
        }
    }

    fn base_state(engine: Engine) -> AppState {
        let store = Arc::new(InMemoryStore::new());
        Arc::new(InMemoryStore::new());
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
            Arc::new(engine),
            jwt_config,
            "test-worker-token".to_string(),
            event_sender,
        )
    }

    fn test_state() -> AppState {
        let store = Arc::new(InMemoryStore::new());
        let provider = Arc::new(ClaudeCodeProvider::new());
        let mut engine = Engine::new(store.clone(), provider);
        engine.register(TestWorkflow).unwrap();
        engine.register(AnotherWorkflow).unwrap();
        base_state(engine)
    }

    fn test_state_with_categories() -> AppState {
        let store = Arc::new(InMemoryStore::new());
        let provider = Arc::new(ClaudeCodeProvider::new());
        let mut engine = Engine::new(store.clone(), provider);
        engine.register(TestWorkflow).unwrap();
        engine.register(EtlNightlyWorkflow).unwrap();
        engine.register(ReportsDailyWorkflow).unwrap();
        base_state(engine)
    }

    async fn run_request(state: AppState, uri: &str) -> (StatusCode, Vec<WorkflowSummary>) {
        let auth_header = make_auth_header(&state);
        let app = Router::new()
            .route("/", get(list_workflows))
            .with_state(state);

        let req = Request::builder()
            .uri(uri)
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        let summaries: Vec<WorkflowSummary> = from_value(json_val["data"].clone()).unwrap();
        (status, summaries)
    }

    #[tokio::test]
    async fn list_workflows_empty() {
        let store = Arc::new(InMemoryStore::new());
        let provider = Arc::new(ClaudeCodeProvider::new());
        let engine = Engine::new(store.clone(), provider);
        let state = base_state(engine);

        let (status, summaries) = run_request(state, "/").await;
        assert_eq!(status, StatusCode::OK);
        assert!(summaries.is_empty());
    }

    #[tokio::test]
    async fn list_workflows_multiple_returns_summaries() {
        let state = test_state();
        let (status, summaries) = run_request(state, "/").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(summaries.len(), 2);
        assert!(summaries.iter().any(|s| s.name == "test-workflow"));
        assert!(summaries.iter().any(|s| s.name == "another-workflow"));
        assert!(summaries.iter().all(|s| s.category.is_none()));
    }

    #[tokio::test]
    async fn list_workflows_filtered_by_name() {
        let state = test_state();
        let (_, summaries) = run_request(state, "/?name=test").await;
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].name, "test-workflow");
    }

    #[tokio::test]
    async fn list_workflows_filter_name_case_insensitive() {
        let state = test_state();
        let (_, summaries) = run_request(state, "/?name=TEST").await;
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].name, "test-workflow");
    }

    #[tokio::test]
    async fn list_workflows_filter_no_match() {
        let state = test_state();
        let (_, summaries) = run_request(state, "/?name=nonexistent").await;
        assert!(summaries.is_empty());
    }

    #[tokio::test]
    async fn list_workflows_returns_category_when_present() {
        let state = test_state_with_categories();
        let (_, summaries) = run_request(state, "/").await;
        let etl = summaries.iter().find(|s| s.name == "etl-nightly").unwrap();
        assert_eq!(etl.category.as_deref(), Some("data/etl"));
        let test = summaries
            .iter()
            .find(|s| s.name == "test-workflow")
            .unwrap();
        assert!(test.category.is_none());
    }

    #[tokio::test]
    async fn list_workflows_filter_by_category_partial() {
        let state = test_state_with_categories();
        let (_, summaries) = run_request(state, "/?category=data").await;
        assert_eq!(summaries.len(), 2);
        assert!(
            summaries
                .iter()
                .all(|s| { s.category.as_deref().is_some_and(|c| c.contains("data")) })
        );
    }

    #[tokio::test]
    async fn list_workflows_filter_by_category_nested_substring() {
        let state = test_state_with_categories();
        let (_, summaries) = run_request(state, "/?category=etl").await;
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].name, "etl-nightly");
    }

    #[tokio::test]
    async fn list_workflows_filter_by_category_case_insensitive() {
        let state = test_state_with_categories();
        let (_, summaries) = run_request(state, "/?category=DATA").await;
        assert_eq!(summaries.len(), 2);
        let (_, summaries) = run_request(test_state_with_categories(), "/?category=Etl").await;
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].name, "etl-nightly");
    }

    #[tokio::test]
    async fn list_workflows_filter_by_category_no_match_excludes_uncategorized() {
        let state = test_state_with_categories();
        let (_, summaries) = run_request(state, "/?category=nonexistent").await;
        assert!(summaries.is_empty());
    }

    #[tokio::test]
    async fn list_workflows_filter_uncategorized_sentinel() {
        let state = test_state_with_categories();
        let (_, summaries) = run_request(state, "/?category=__uncategorized__").await;
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].name, "test-workflow");
    }
}
