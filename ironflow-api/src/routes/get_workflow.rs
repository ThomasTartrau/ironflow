//! `GET /api/v1/workflows/:name` — Get workflow details.

use std::collections::{HashMap, HashSet};

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use ironflow_auth::extractor::Authenticated;
use serde::Serialize;
use serde_json::Value;

use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Sub-workflow detail included in the workflow response.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Serialize)]
pub struct SubWorkflowDetail {
    /// Sub-workflow name.
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Optional Rust source code of the handler.
    pub source_code: Option<String>,
}

/// Workflow detail response.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Serialize)]
pub struct WorkflowDetailResponse {
    /// Workflow name.
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Optional Rust source code of the handler.
    pub source_code: Option<String>,
    /// Sub-workflows invoked by this handler (recursive, depth-limited).
    pub sub_workflows: Vec<SubWorkflowDetail>,
    /// Optional `/`-separated category path used to group workflows.
    pub category: Option<String>,
    /// Current handler version.
    pub version: Option<String>,
    /// JSON Schema describing the expected input payload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<Value>,
    /// Labels automatically applied to every run of this workflow.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub default_labels: HashMap<String, String>,
    /// Optional 6-field cron expression for automatic execution.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schedule: Option<String>,
}

/// Get details about a registered workflow.
///
/// # Errors
///
/// - 404 if the workflow is not registered
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/workflows/{name}",
        tags = ["workflows"],
        params(("name" = String, Path, description = "Workflow name")),
        responses(
            (status = 200, description = "Workflow details", body = WorkflowDetailResponse),
            (status = 401, description = "Unauthorized"),
            (status = 404, description = "Workflow not found")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn get_workflow(
    _auth: Authenticated,
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let info = state
        .engine
        .handler_info(&name)
        .ok_or_else(|| ApiError::WorkflowNotFound(name.clone()))?;

    let mut sub_workflows = Vec::new();
    let mut visited = HashSet::new();
    visited.insert(name.clone());
    collect_sub_workflows(
        &state,
        &info.sub_workflows,
        &mut sub_workflows,
        &mut visited,
        5,
    );

    Ok(ok(WorkflowDetailResponse {
        name,
        description: info.description,
        source_code: info.source_code,
        sub_workflows,
        category: info.category,
        version: info.version,
        input_schema: info.input_schema,
        default_labels: info.default_labels,
        schedule: info.schedule.map(|s| s.as_str().to_string()),
    }))
}

fn collect_sub_workflows(
    state: &AppState,
    names: &[String],
    result: &mut Vec<SubWorkflowDetail>,
    visited: &mut HashSet<String>,
    depth: usize,
) {
    if depth == 0 {
        return;
    }
    for sub_name in names {
        if !visited.insert(sub_name.clone()) {
            continue;
        }
        if let Some(sub_info) = state.engine.handler_info(sub_name) {
            collect_sub_workflows(state, &sub_info.sub_workflows, result, visited, depth - 1);
            result.push(SubWorkflowDetail {
                name: sub_name.clone(),
                description: sub_info.description,
                source_code: sub_info.source_code,
            });
        }
    }
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
    use serde_json::Value as JsonValue;
    use std::sync::Arc;
    use tokio::sync::broadcast;
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::*;

    struct DescribedWorkflow;
    impl WorkflowHandler for DescribedWorkflow {
        fn name(&self) -> &str {
            "my-workflow"
        }
        fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
            Box::pin(async move { Ok(()) })
        }
    }

    struct CategorizedWorkflow;
    impl WorkflowHandler for CategorizedWorkflow {
        fn name(&self) -> &str {
            "cat-workflow"
        }
        fn category(&self) -> Option<&str> {
            Some("data/etl")
        }
        fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
            Box::pin(async move { Ok(()) })
        }
    }

    fn test_state() -> AppState {
        let store = Arc::new(InMemoryStore::new());
        Arc::new(InMemoryStore::new());
        let provider = Arc::new(ClaudeCodeProvider::new());
        let mut engine = Engine::new(store.clone(), provider);
        engine.register(DescribedWorkflow).unwrap();
        engine.register(CategorizedWorkflow).unwrap();
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

    fn make_auth_header(state: &AppState) -> String {
        let user_id = Uuid::now_v7();
        let token = AccessToken::for_user(user_id, "testuser", false, &state.jwt_config).unwrap();
        format!("Bearer {}", token.0)
    }

    #[tokio::test]
    async fn get_workflow_found() {
        let state = test_state();
        let auth_header = make_auth_header(&state);
        let app = Router::new()
            .route("/{name}", get(get_workflow))
            .with_state(state);

        let req = Request::builder()
            .uri("/my-workflow")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = serde_json::from_slice(&body).unwrap();
        assert_eq!(json_val["data"]["name"], "my-workflow");
    }

    #[tokio::test]
    async fn get_workflow_returns_category_when_set() {
        let state = test_state();
        let auth_header = make_auth_header(&state);
        let app = Router::new()
            .route("/{name}", get(get_workflow))
            .with_state(state);

        let req = Request::builder()
            .uri("/cat-workflow")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = serde_json::from_slice(&body).unwrap();
        assert_eq!(json_val["data"]["category"], "data/etl");
    }

    #[tokio::test]
    async fn get_workflow_category_null_when_uncategorized() {
        let state = test_state();
        let auth_header = make_auth_header(&state);
        let app = Router::new()
            .route("/{name}", get(get_workflow))
            .with_state(state);

        let req = Request::builder()
            .uri("/my-workflow")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = serde_json::from_slice(&body).unwrap();
        assert!(json_val["data"]["category"].is_null());
    }

    struct ScheduledWorkflow {
        schedule: ironflow_engine::prelude::CronSchedule,
    }
    impl ScheduledWorkflow {
        fn new() -> Self {
            Self {
                schedule: ironflow_engine::prelude::CronSchedule::new("0 0 * * * *").unwrap(),
            }
        }
    }
    impl WorkflowHandler for ScheduledWorkflow {
        fn name(&self) -> &str {
            "sched-workflow"
        }
        fn schedule(&self) -> Option<&ironflow_engine::prelude::CronSchedule> {
            Some(&self.schedule)
        }
        fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
            Box::pin(async move { Ok(()) })
        }
    }

    fn test_state_with_schedule() -> AppState {
        let store = Arc::new(InMemoryStore::new());
        let provider = Arc::new(ClaudeCodeProvider::new());
        let mut engine = Engine::new(store.clone(), provider);
        engine.register(DescribedWorkflow).unwrap();
        engine.register(ScheduledWorkflow::new()).unwrap();
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

    #[tokio::test]
    async fn get_workflow_returns_schedule_when_set() {
        let state = test_state_with_schedule();
        let auth_header = make_auth_header(&state);
        let app = Router::new()
            .route("/{name}", get(get_workflow))
            .with_state(state);

        let req = Request::builder()
            .uri("/sched-workflow")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = serde_json::from_slice(&body).unwrap();
        assert_eq!(json_val["data"]["schedule"], "0 0 * * * *");
    }

    #[tokio::test]
    async fn get_workflow_schedule_null_when_unscheduled() {
        let state = test_state_with_schedule();
        let auth_header = make_auth_header(&state);
        let app = Router::new()
            .route("/{name}", get(get_workflow))
            .with_state(state);

        let req = Request::builder()
            .uri("/my-workflow")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = serde_json::from_slice(&body).unwrap();
        assert!(json_val["data"]["schedule"].is_null());
    }

    #[tokio::test]
    async fn get_workflow_not_found() {
        let state = test_state();
        let auth_header = make_auth_header(&state);
        let app = Router::new()
            .route("/{name}", get(get_workflow))
            .with_state(state);

        let req = Request::builder()
            .uri("/nonexistent")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
