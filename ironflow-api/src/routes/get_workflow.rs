//! `GET /api/v1/workflows/:name` — Get workflow details.

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use ironflow_auth::extractor::AuthenticatedUser;
use serde::Serialize;

use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Sub-workflow detail included in the workflow response.
#[derive(Debug, Serialize)]
struct SubWorkflowDetail {
    /// Sub-workflow name.
    name: String,
    /// Human-readable description.
    description: String,
    /// Optional Rust source code of the handler.
    source_code: Option<String>,
}

/// Workflow detail response.
#[derive(Debug, Serialize)]
struct WorkflowDetailResponse {
    /// Workflow name.
    name: String,
    /// Human-readable description.
    description: String,
    /// Optional Rust source code of the handler.
    source_code: Option<String>,
    /// Sub-workflows invoked by this handler (recursive, depth-limited).
    sub_workflows: Vec<SubWorkflowDetail>,
}

/// Get details about a registered workflow.
///
/// # Errors
///
/// - 404 if the workflow is not registered
pub async fn get_workflow(
    _user: AuthenticatedUser,
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let info = state
        .engine
        .handler_info(&name)
        .ok_or_else(|| ApiError::WorkflowNotFound(name.clone()))?;

    let mut sub_workflows = Vec::new();
    let mut visited = std::collections::HashSet::new();
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
    }))
}

fn collect_sub_workflows(
    state: &AppState,
    names: &[String],
    result: &mut Vec<SubWorkflowDetail>,
    visited: &mut std::collections::HashSet<String>,
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
    use ironflow_store::memory::InMemoryStore;
    use std::sync::Arc;
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

    fn test_state() -> AppState {
        let store = Arc::new(InMemoryStore::new());
        let user_store: Arc<dyn ironflow_store::user_store::UserStore> =
            Arc::new(InMemoryStore::new());
        let provider = Arc::new(ClaudeCodeProvider::new());
        let mut engine = Engine::new(store.clone(), provider);
        engine.register(DescribedWorkflow).unwrap();
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
            engine: Arc::new(engine),
            jwt_config,
            worker_token: "test-worker-token".to_string(),
        }
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
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["data"]["name"], "my-workflow");
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
