//! `GET /api/v1/api-keys/scopes` -- List available scopes for the current user.

use axum::response::IntoResponse;
use ironflow_auth::extractor::AuthenticatedUser;
use ironflow_store::entities::ApiKeyScope;
use serde::Serialize;

use crate::error::ApiError;
use crate::response::ok;

/// A scope entry with its machine name and human-readable label.
#[derive(Debug, Serialize)]
pub struct ScopeEntry {
    /// Machine-readable scope value (e.g. "runs_read").
    pub value: String,
    /// Human-readable label (e.g. "Runs Read").
    pub label: String,
    /// Short description.
    pub description: String,
}

/// Return the list of scopes the current user is allowed to assign to API keys.
///
/// Admins get all scopes, members only get read-only scopes.
pub async fn available_scopes(user: AuthenticatedUser) -> Result<impl IntoResponse, ApiError> {
    let scopes = if user.is_admin {
        ApiKeyScope::all_non_admin()
    } else {
        ApiKeyScope::member_allowed().to_vec()
    };

    let entries: Vec<ScopeEntry> = scopes
        .into_iter()
        .map(|s| {
            let (label, description) = scope_metadata(&s);
            ScopeEntry {
                value: s.to_string(),
                label: label.to_string(),
                description: description.to_string(),
            }
        })
        .collect();

    Ok(ok(entries))
}

fn scope_metadata(scope: &ApiKeyScope) -> (&'static str, &'static str) {
    match scope {
        ApiKeyScope::WorkflowsRead => ("Workflows Read", "Read workflow definitions"),
        ApiKeyScope::RunsRead => ("Runs Read", "Read runs and their steps"),
        ApiKeyScope::RunsWrite => ("Runs Write", "Create new runs"),
        ApiKeyScope::RunsManage => ("Runs Manage", "Cancel, approve, reject, retry runs"),
        ApiKeyScope::StatsRead => ("Stats Read", "Read aggregated statistics"),
        ApiKeyScope::Admin => ("Admin", "Full access to all operations"),
    }
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use http_body_util::BodyExt;
    use ironflow_auth::jwt::{AccessToken, JwtConfig};
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::context::WorkflowContext;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
    use ironflow_store::api_key_store::ApiKeyStore;
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::user_store::UserStore;
    use serde_json::Value as JsonValue;
    use std::sync::Arc;
    use tower::ServiceExt;
    use uuid::Uuid;

    use crate::state::AppState;

    use super::*;

    struct TestWorkflow;

    impl WorkflowHandler for TestWorkflow {
        fn name(&self) -> &str {
            "test-workflow"
        }

        fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
            Box::pin(async move { Ok(()) })
        }
    }

    fn test_jwt_config() -> Arc<JwtConfig> {
        Arc::new(JwtConfig {
            secret: "test-secret-for-scope-tests".to_string(),
            access_token_ttl_secs: 900,
            refresh_token_ttl_secs: 604800,
            cookie_domain: None,
            cookie_secure: false,
        })
    }

    fn test_state() -> AppState {
        let store = Arc::new(InMemoryStore::new());
        let user_store: Arc<dyn UserStore> = Arc::new(InMemoryStore::new());
        let api_key_store: Arc<dyn ApiKeyStore> = Arc::new(InMemoryStore::new());
        let provider = Arc::new(ClaudeCodeProvider::new());
        let mut engine = Engine::new(store.clone(), provider);
        engine
            .register(TestWorkflow)
            .expect("failed to register test workflow");
        AppState::new(
            store,
            user_store,
            api_key_store,
            Arc::new(engine),
            test_jwt_config(),
            "test-worker-token".to_string(),
        )
    }

    fn make_auth_header(is_admin: bool, state: &AppState) -> String {
        let user_id = Uuid::now_v7();
        let token =
            AccessToken::for_user(user_id, "testuser", is_admin, &state.jwt_config).unwrap();
        format!("Bearer {}", token.0)
    }

    #[tokio::test]
    async fn admin_gets_all_non_admin_scopes() {
        let state = test_state();
        let auth_header = make_auth_header(true, &state);
        let app = Router::new()
            .route("/", get(available_scopes))
            .with_state(state);

        let req = Request::builder()
            .uri("/")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: JsonValue = serde_json::from_slice(&body).unwrap();
        let scopes = json["data"].as_array().unwrap();
        assert_eq!(scopes.len(), 5);

        let values: Vec<&str> = scopes
            .iter()
            .map(|s| s["value"].as_str().unwrap())
            .collect();
        assert!(values.contains(&"runs_write"));
        assert!(values.contains(&"runs_manage"));
    }

    #[tokio::test]
    async fn member_gets_read_only_scopes() {
        let state = test_state();
        let auth_header = make_auth_header(false, &state);
        let app = Router::new()
            .route("/", get(available_scopes))
            .with_state(state);

        let req = Request::builder()
            .uri("/")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: JsonValue = serde_json::from_slice(&body).unwrap();
        let scopes = json["data"].as_array().unwrap();
        assert_eq!(scopes.len(), 3);

        let values: Vec<&str> = scopes
            .iter()
            .map(|s| s["value"].as_str().unwrap())
            .collect();
        assert!(values.contains(&"workflows_read"));
        assert!(values.contains(&"runs_read"));
        assert!(values.contains(&"stats_read"));
        assert!(!values.contains(&"runs_write"));
        assert!(!values.contains(&"runs_manage"));
    }
}
