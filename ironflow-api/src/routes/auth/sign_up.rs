//! `POST /api/v1/auth/sign-up` — Register a new user.

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::IntoResponse;

use validator::Validate;

use ironflow_auth::cookies::{build_auth_cookie, build_refresh_cookie};
use ironflow_auth::jwt::{AccessToken, RefreshToken};
use ironflow_auth::password;
use ironflow_store::entities::NewUser;
use ironflow_store::error::StoreError;

use crate::entities::SignUpRequest;
use crate::error::ApiError;
use crate::state::AppState;

/// Register a new user with email and password.
///
/// Returns access and refresh tokens on success, and sets HttpOnly cookies.
///
/// # Errors
///
/// - 400 if email/username/password is invalid
/// - 409 if email or username is already taken
pub async fn sign_up(
    State(state): State<AppState>,
    Json(req): Json<SignUpRequest>,
) -> Result<impl IntoResponse, ApiError> {
    req.validate()
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    let hash =
        password::hash(&req.password).map_err(|_| ApiError::Internal("hashing failed".into()))?;

    let user = state
        .user_store
        .create_user(NewUser {
            email: req.email,
            username: req.username,
            password_hash: hash,
        })
        .await
        .map_err(|e| match e {
            StoreError::DuplicateEmail(_) => ApiError::DuplicateEmail,
            StoreError::DuplicateUsername(_) => ApiError::DuplicateUsername,
            other => ApiError::Store(other),
        })?;

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

    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::context::WorkflowContext;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
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
        let provider = Arc::new(ClaudeCodeProvider::new());
        let mut engine = Engine::new(store.clone(), provider);
        engine
            .register(TestWorkflow)
            .expect("failed to register test workflow");
        AppState::new(
            store,
            user_store,
            Arc::new(engine),
            test_jwt_config(),
            "test-worker-token".to_string(),
        )
    }

    #[tokio::test]
    async fn sign_up_success() {
        let state = test_state();
        let app = Router::new().route("/", post(sign_up)).with_state(state);

        let req = Request::builder()
            .uri("/")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(
                to_string(&json!({
                    "email": "test@example.com",
                    "username": "testuser",
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
    async fn sign_up_invalid_email() {
        let state = test_state();
        let app = Router::new().route("/", post(sign_up)).with_state(state);

        let req = Request::builder()
            .uri("/")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(
                to_string(&json!({
                    "email": "invalid-email",
                    "username": "testuser",
                    "password": "password123"
                }))
                .expect("failed to serialize"),
            ))
            .expect("failed to build request");

        let resp = app.oneshot(req).await.expect("request failed");
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn sign_up_username_too_short() {
        let state = test_state();
        let app = Router::new().route("/", post(sign_up)).with_state(state);

        let req = Request::builder()
            .uri("/")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(
                to_string(&json!({
                    "email": "test@example.com",
                    "username": "ab",
                    "password": "password123"
                }))
                .expect("failed to serialize"),
            ))
            .expect("failed to build request");

        let resp = app.oneshot(req).await.expect("request failed");
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn sign_up_password_too_short() {
        let state = test_state();
        let app = Router::new().route("/", post(sign_up)).with_state(state);

        let req = Request::builder()
            .uri("/")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(
                to_string(&json!({
                    "email": "test@example.com",
                    "username": "testuser",
                    "password": "short"
                }))
                .expect("failed to serialize"),
            ))
            .expect("failed to build request");

        let resp = app.oneshot(req).await.expect("request failed");
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn sign_up_duplicate_email() {
        let state = test_state();
        let app = Router::new().route("/", post(sign_up)).with_state(state);

        let first_req = Request::builder()
            .uri("/")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(
                to_string(&json!({
                    "email": "test@example.com",
                    "username": "testuser1",
                    "password": "password123"
                }))
                .expect("failed to serialize"),
            ))
            .expect("failed to build request");

        let resp = app
            .clone()
            .oneshot(first_req)
            .await
            .expect("first request failed");
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        let second_req = Request::builder()
            .uri("/")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(
                to_string(&json!({
                    "email": "test@example.com",
                    "username": "testuser2",
                    "password": "password123"
                }))
                .expect("failed to serialize"),
            ))
            .expect("failed to build request");

        let resp = app
            .oneshot(second_req)
            .await
            .expect("second request failed");
        assert_eq!(resp.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn sign_up_duplicate_username() {
        let state = test_state();
        let app = Router::new().route("/", post(sign_up)).with_state(state);

        let first_req = Request::builder()
            .uri("/")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(
                to_string(&json!({
                    "email": "test1@example.com",
                    "username": "testuser",
                    "password": "password123"
                }))
                .expect("failed to serialize"),
            ))
            .expect("failed to build request");

        let resp = app
            .clone()
            .oneshot(first_req)
            .await
            .expect("first request failed");
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        let second_req = Request::builder()
            .uri("/")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(
                to_string(&json!({
                    "email": "test2@example.com",
                    "username": "testuser",
                    "password": "password123"
                }))
                .expect("failed to serialize"),
            ))
            .expect("failed to build request");

        let resp = app
            .oneshot(second_req)
            .await
            .expect("second request failed");
        assert_eq!(resp.status(), StatusCode::CONFLICT);
    }
}
