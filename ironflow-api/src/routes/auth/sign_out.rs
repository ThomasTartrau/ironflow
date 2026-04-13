//! `POST /api/v1/auth/sign-out` — Clear auth cookies.

use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue};
use axum::response::IntoResponse;

use serde_json::json;

use ironflow_auth::cookies::{clear_auth_cookie, clear_refresh_cookie};
use ironflow_auth::extractor::AuthenticatedUser;

use crate::response::ok;
use crate::state::AppState;

/// Sign out the current user by clearing auth cookies.
pub async fn sign_out(
    State(state): State<AppState>,
    _user: AuthenticatedUser,
) -> impl IntoResponse {
    let mut headers = HeaderMap::new();
    if let Ok(val) = HeaderValue::from_str(&clear_auth_cookie(&state.jwt_config)) {
        headers.append("Set-Cookie", val);
    }
    if let Ok(val) = HeaderValue::from_str(&clear_refresh_cookie(&state.jwt_config)) {
        headers.append("Set-Cookie", val);
    }

    (headers, ok(json!({ "signed_out": true })))
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::post;
    use ironflow_auth::jwt::{AccessToken, JwtConfig};
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::context::WorkflowContext;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
    use ironflow_store::api_key_store::ApiKeyStore;
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::user_store::UserStore;
    use std::sync::Arc;
    use tower::ServiceExt;
    use uuid::Uuid;

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
            secret: "test-secret-for-auth-tests".to_string(),
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

    fn make_auth_header(state: &AppState) -> String {
        let user_id = Uuid::now_v7();
        let token = AccessToken::for_user(user_id, "testuser", false, &state.jwt_config)
            .expect("failed to create token");
        format!("Bearer {}", token.0)
    }

    #[tokio::test]
    async fn sign_out_success() {
        let state = test_state();
        let auth_header = make_auth_header(&state);
        let app = Router::new().route("/", post(sign_out)).with_state(state);

        let req = Request::builder()
            .uri("/")
            .method("POST")
            .header("authorization", auth_header)
            .body(Body::empty())
            .expect("failed to build request");

        let resp = app.oneshot(req).await.expect("request failed");
        assert_eq!(resp.status(), StatusCode::OK);

        let set_cookie = resp.headers().get_all("set-cookie");
        assert!(set_cookie.iter().count() > 0);
    }

    #[tokio::test]
    async fn sign_out_without_token() {
        let state = test_state();
        let app = Router::new().route("/", post(sign_out)).with_state(state);

        let req = Request::builder()
            .uri("/")
            .method("POST")
            .body(Body::empty())
            .expect("failed to build request");

        let resp = app.oneshot(req).await.expect("request failed");
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}
