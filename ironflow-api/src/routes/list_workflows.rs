//! `GET /api/v1/workflows` — List registered workflows.

use axum::extract::{Query, State};
use axum::response::IntoResponse;
use ironflow_auth::extractor::AuthenticatedUser;
use serde::Deserialize;

use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Query parameters for listing workflows.
#[derive(Debug, Deserialize)]
pub struct ListWorkflowsQuery {
    /// Optional case-insensitive partial match on workflow name.
    pub name: Option<String>,
}

/// List registered workflow names, optionally filtered by name.
///
/// # Query Parameters
///
/// - `name` — Filter by workflow name, case-insensitive partial match (optional)
pub async fn list_workflows(
    _user: AuthenticatedUser,
    State(state): State<AppState>,
    Query(params): Query<ListWorkflowsQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let mut names: Vec<String> = state
        .engine
        .handler_names()
        .into_iter()
        .map(|s| s.to_string())
        .collect();

    if let Some(ref filter) = params.name {
        let lower = filter.to_lowercase();
        names.retain(|n| n.to_lowercase().contains(&lower));
    }

    Ok(ok(names))
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
    use serde_json::{Value as JsonValue, from_slice, from_value};
    use std::sync::Arc;
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

    fn test_state() -> AppState {
        let store = Arc::new(InMemoryStore::new());
        let user_store: Arc<dyn ironflow_store::user_store::UserStore> =
            Arc::new(InMemoryStore::new());
        let provider = Arc::new(ClaudeCodeProvider::new());
        let mut engine = Engine::new(store.clone(), provider);
        engine.register(TestWorkflow).unwrap();
        engine.register(AnotherWorkflow).unwrap();
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

    #[tokio::test]
    async fn list_workflows_empty() {
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
        let state = AppState {
            store,
            user_store,
            engine,
            jwt_config,
            worker_token: "test-worker-token".to_string(),
        };
        let auth_header = make_auth_header(&state);

        let app = Router::new()
            .route("/", get(list_workflows))
            .with_state(state);

        let req = Request::builder()
            .uri("/")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        assert_eq!(json_val["data"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn list_workflows_multiple() {
        let state = test_state();
        let auth_header = make_auth_header(&state);
        let app = Router::new()
            .route("/", get(list_workflows))
            .with_state(state);

        let req = Request::builder()
            .uri("/")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        let workflows: Vec<String> = from_value(json_val["data"].clone()).unwrap();
        assert_eq!(workflows.len(), 2);
        assert!(workflows.contains(&"test-workflow".to_string()));
        assert!(workflows.contains(&"another-workflow".to_string()));
    }

    #[tokio::test]
    async fn list_workflows_filtered_by_name() {
        let state = test_state();
        let auth_header = make_auth_header(&state);
        let app = Router::new()
            .route("/", get(list_workflows))
            .with_state(state);

        let req = Request::builder()
            .uri("/?name=test")
            .header("authorization", &auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        let workflows: Vec<String> = from_value(json_val["data"].clone()).unwrap();
        assert_eq!(workflows.len(), 1);
        assert_eq!(workflows[0], "test-workflow");
    }

    #[tokio::test]
    async fn list_workflows_filter_case_insensitive() {
        let state = test_state();
        let auth_header = make_auth_header(&state);
        let app = Router::new()
            .route("/", get(list_workflows))
            .with_state(state);

        let req = Request::builder()
            .uri("/?name=TEST")
            .header("authorization", &auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        let workflows: Vec<String> = from_value(json_val["data"].clone()).unwrap();
        assert_eq!(workflows.len(), 1);
        assert_eq!(workflows[0], "test-workflow");
    }

    #[tokio::test]
    async fn list_workflows_filter_no_match() {
        let state = test_state();
        let auth_header = make_auth_header(&state);
        let app = Router::new()
            .route("/", get(list_workflows))
            .with_state(state);

        let req = Request::builder()
            .uri("/?name=nonexistent")
            .header("authorization", &auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        let workflows: Vec<String> = from_value(json_val["data"].clone()).unwrap();
        assert!(workflows.is_empty());
    }
}
