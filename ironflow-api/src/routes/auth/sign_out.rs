//! `POST /api/v1/auth/sign-out` — Clear auth cookies.

use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue};
use axum::response::IntoResponse;

use serde_json::json;

use ironflow_auth::cookies::{clear_auth_cookie, clear_refresh_cookie};
use ironflow_auth::extractor::AuthenticatedUser;
use ironflow_store::error::StoreError;

use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Sign out the current user by clearing auth cookies.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        post,
        path = "/api/v1/auth/sign-out",
        tags = ["auth"],
        responses(
            (status = 200, description = "User signed out successfully, cookies cleared"),
            (status = 401, description = "Unauthorized")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn sign_out(
    State(state): State<AppState>,
    user: AuthenticatedUser,
) -> Result<impl IntoResponse, ApiError> {
    // Revoke every session of the user: the access token presented here and
    // any copy of it stop working, and stored refresh tokens are dropped. A
    // token whose user no longer exists has nothing left to revoke.
    match state.store.revoke_user_sessions(user.user_id).await {
        Ok(_) | Err(StoreError::UserNotFound(_)) => {}
        Err(e) => return Err(ApiError::Store(e)),
    }

    let mut headers = HeaderMap::new();
    if let Ok(val) = HeaderValue::from_str(&clear_auth_cookie(&state.jwt_config)) {
        headers.append("Set-Cookie", val);
    }
    if let Ok(val) = HeaderValue::from_str(&clear_refresh_cookie(&state.jwt_config)) {
        headers.append("Set-Cookie", val);
    }

    Ok((headers, ok(json!({ "signed_out": true }))))
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::{get, post};
    use ironflow_auth::jwt::{AccessToken, JwtConfig};
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::context::WorkflowContext;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
    use ironflow_engine::notify::Event;
    use ironflow_store::entities::NewUser;
    use ironflow_store::memory::InMemoryStore;
    use std::sync::Arc;
    use tokio::sync::broadcast;
    use tower::ServiceExt;

    use super::*;
    use crate::routes::auth::me::me;
    use crate::routes::test_helpers::create_user_auth_header;

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
        let provider = Arc::new(ClaudeCodeProvider::new());
        let mut engine = Engine::new(store.clone(), provider);
        engine
            .register(TestWorkflow)
            .expect("failed to register test workflow");
        let (event_sender, _) = broadcast::channel::<Event>(1);
        AppState::new(
            store,
            Arc::new(engine),
            test_jwt_config(),
            "test-worker-token".to_string(),
            event_sender,
        )
    }

    #[tokio::test]
    async fn sign_out_success() {
        let state = test_state();
        let auth_header = create_user_auth_header(&state, "testuser", false).await;
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

    #[tokio::test]
    async fn access_token_rejected_after_sign_out() {
        let state = test_state();
        let user = state
            .store
            .create_user(NewUser {
                email: "signout@example.com".to_string(),
                username: "signoutuser".to_string(),
                password_hash: "argon2hash".to_string(),
                is_admin: Some(false),
            })
            .await
            .expect("failed to create user");
        let token = AccessToken::for_user(user.id, "signoutuser", false, &state.jwt_config)
            .expect("failed to create token");
        let auth_header = format!("Bearer {}", token.0);
        let app = Router::new()
            .route("/me", get(me))
            .route("/sign-out", post(sign_out))
            .with_state(state);

        let send = |method: &str, uri: &str| {
            Request::builder()
                .uri(uri)
                .method(method)
                .header("authorization", &auth_header)
                .body(Body::empty())
                .expect("failed to build request")
        };

        let before = app
            .clone()
            .oneshot(send("GET", "/me"))
            .await
            .expect("request failed");
        assert_eq!(before.status(), StatusCode::OK);

        let signed_out = app
            .clone()
            .oneshot(send("POST", "/sign-out"))
            .await
            .expect("request failed");
        assert_eq!(signed_out.status(), StatusCode::OK);

        // The token is still correctly signed and unexpired, but the session
        // it belonged to was revoked server-side.
        let after = app
            .clone()
            .oneshot(send("GET", "/me"))
            .await
            .expect("request failed");
        assert_eq!(after.status(), StatusCode::UNAUTHORIZED);

        let replay = app
            .oneshot(send("POST", "/sign-out"))
            .await
            .expect("request failed");
        assert_eq!(replay.status(), StatusCode::UNAUTHORIZED);
    }
}
