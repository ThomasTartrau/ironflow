//! `POST /api/v1/auth/sign-in` — Authenticate with email and password.

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::IntoResponse;

use ironflow_auth::cookies::{build_auth_cookie, build_refresh_cookie};
use ironflow_auth::jwt::{AccessToken, RefreshToken};
use ironflow_auth::password;

use crate::entities::SignInRequest;
use crate::error::ApiError;
use crate::state::AppState;

/// Authenticate a user with email and password.
///
/// Returns access and refresh tokens on success, and sets HttpOnly cookies.
///
/// # Errors
///
/// - 401 if email not found or password is wrong
pub async fn sign_in(
    State(state): State<AppState>,
    Json(req): Json<SignInRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let user = state
        .user_store
        .find_user_by_email(&req.email)
        .await?
        .ok_or(ApiError::InvalidCredentials)?;

    let valid = password::verify(&req.password, &user.password_hash)
        .map_err(|_| ApiError::Internal("password verification failed".into()))?;

    if !valid {
        return Err(ApiError::InvalidCredentials);
    }

    let access = AccessToken::for_user(user.id, &user.username, user.is_admin, &state.jwt_config)
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let refresh = RefreshToken::for_user(user.id, &user.username, user.is_admin, &state.jwt_config)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let mut headers = HeaderMap::new();
    if let Ok(val) = HeaderValue::from_str(&build_auth_cookie(&access.0, &state.jwt_config)) {
        headers.append("Set-Cookie", val);
    }
    if let Ok(val) = HeaderValue::from_str(&build_refresh_cookie(&refresh.0, &state.jwt_config)) {
        headers.append("Set-Cookie", val);
    }

    Ok((StatusCode::NO_CONTENT, headers))
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::post;
    use ironflow_auth::jwt::JwtConfig;
    use ironflow_auth::password;
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::context::WorkflowContext;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
    use ironflow_store::api_key_store::ApiKeyStore;
    use ironflow_store::entities::NewUser;
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::user_store::UserStore;
    use serde_json::{json, to_string};
    use std::sync::Arc;
    use tower::ServiceExt;

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

    #[tokio::test]
    async fn sign_in_success() {
        let state = test_state();
        let hash = password::hash("password123").expect("failed to hash password");
        state
            .user_store
            .create_user(NewUser {
                email: "test@example.com".to_string(),
                username: "testuser".to_string(),
                password_hash: hash,
            })
            .await
            .expect("failed to create user");

        let app = Router::new().route("/", post(sign_in)).with_state(state);

        let req = Request::builder()
            .uri("/")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(
                to_string(&json!({
                    "email": "test@example.com",
                    "password": "password123"
                }))
                .expect("failed to serialize"),
            ))
            .expect("failed to build request");

        let resp = app.oneshot(req).await.expect("request failed");
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        let set_cookie = resp.headers().get_all("set-cookie");
        assert!(set_cookie.iter().count() > 0);
    }

    #[tokio::test]
    async fn sign_in_unknown_email() {
        let state = test_state();
        let app = Router::new().route("/", post(sign_in)).with_state(state);

        let req = Request::builder()
            .uri("/")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(
                to_string(&json!({
                    "email": "unknown@example.com",
                    "password": "password123"
                }))
                .expect("failed to serialize"),
            ))
            .expect("failed to build request");

        let resp = app.oneshot(req).await.expect("request failed");
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn sign_in_wrong_password() {
        let state = test_state();
        let hash = password::hash("password123").expect("failed to hash password");
        state
            .user_store
            .create_user(NewUser {
                email: "test@example.com".to_string(),
                username: "testuser".to_string(),
                password_hash: hash,
            })
            .await
            .expect("failed to create user");

        let app = Router::new().route("/", post(sign_in)).with_state(state);

        let req = Request::builder()
            .uri("/")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(
                to_string(&json!({
                    "email": "test@example.com",
                    "password": "wrongpassword"
                }))
                .expect("failed to serialize"),
            ))
            .expect("failed to build request");

        let resp = app.oneshot(req).await.expect("request failed");
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn sign_in_sets_cookies() {
        let state = test_state();
        let hash = password::hash("password123").expect("failed to hash password");
        state
            .user_store
            .create_user(NewUser {
                email: "test@example.com".to_string(),
                username: "testuser".to_string(),
                password_hash: hash,
            })
            .await
            .expect("failed to create user");

        let app = Router::new().route("/", post(sign_in)).with_state(state);

        let req = Request::builder()
            .uri("/")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(
                to_string(&json!({
                    "email": "test@example.com",
                    "password": "password123"
                }))
                .expect("failed to serialize"),
            ))
            .expect("failed to build request");

        let resp = app.oneshot(req).await.expect("request failed");
        let set_cookie = resp.headers().get_all("set-cookie");

        let cookie_values: Vec<_> = set_cookie.iter().map(|h| h.to_str()).collect();
        assert_eq!(cookie_values.len(), 2);
        assert!(cookie_values[0].is_ok());
        assert!(cookie_values[1].is_ok());
    }
}
