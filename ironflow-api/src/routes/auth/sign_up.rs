//! `POST /api/v1/auth/sign-up` — Register a new user.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use tracing::info;

use validator::Validate;

use ironflow_auth::password;
use ironflow_store::entities::NewUser;
use ironflow_store::error::StoreError;

use crate::entities::SignUpRequest;
use crate::error::ApiError;
use crate::state::AppState;

/// Register a new user with email and password.
///
/// The answer is the same `204` whether the email was free or already
/// registered, and it never carries a session: the client signs in next
/// with the same credentials. Without that, the status code or the presence
/// of session cookies would tell anyone which emails have an account.
///
/// A taken username is still reported (`409`). It is checked before the
/// email, so that answer says nothing about the email.
///
/// # Errors
///
/// - 400 if the email or username is invalid
/// - 400 `WEAK_PASSWORD` if the password breaks the password policy
/// - 409 if the username is already taken
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        post,
        path = "/api/v1/auth/sign-up",
        tags = ["auth"],
        request_body(content = SignUpRequest, description = "Sign up credentials"),
        responses(
            (status = 204, description = "Request accepted. Same answer whether the email was free or already registered; no session is issued, sign in next"),
            (status = 400, description = "Invalid email or username, or password breaks the strength policy (WEAK_PASSWORD)"),
            (status = 409, description = "Username already taken")
        )
    )
)]
pub async fn sign_up(
    State(state): State<AppState>,
    Json(req): Json<SignUpRequest>,
) -> Result<impl IntoResponse, ApiError> {
    req.validate()
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    password::check_strength(&req.password, &[&req.email, &req.username])?;

    if state
        .store
        .find_user_by_username(&req.username)
        .await?
        .is_some()
    {
        return Err(ApiError::DuplicateUsername);
    }

    // Hash on every path, so a taken email costs as much time as a free one.
    let hash =
        password::hash(&req.password).map_err(|_| ApiError::Internal("hashing failed".into()))?;

    let created = state
        .store
        .create_user(NewUser {
            email: req.email,
            username: req.username,
            password_hash: hash,
            is_admin: None,
        })
        .await;

    match created {
        Ok(_) => {}
        Err(StoreError::DuplicateEmail(_)) => {
            info!("sign-up for an already registered email, answered as a success");
        }
        Err(StoreError::DuplicateUsername(_)) => return Err(ApiError::DuplicateUsername),
        Err(other) => return Err(ApiError::Store(other)),
    }

    Ok(StatusCode::NO_CONTENT)
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
    use ironflow_engine::notify::Event;
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::store::Store;
    use serde_json::{json, to_string};
    use std::sync::Arc;
    use tokio::sync::broadcast;
    use tower::ServiceExt;

    use super::*;

    const STRONG_PASSWORD: &str = "correct horse battery staple";

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
        let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
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
                    "password": STRONG_PASSWORD
                }))
                .expect("failed to serialize"),
            ))
            .expect("failed to build request");

        let resp = app.oneshot(req).await.expect("request failed");
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
        // No session: the client signs in next, whatever the email.
        assert_eq!(resp.headers().get_all("set-cookie").iter().count(), 0);
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
                    "password": STRONG_PASSWORD
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
                    "password": STRONG_PASSWORD
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
    async fn sign_up_duplicate_email_answers_like_a_new_email() {
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
                    "password": STRONG_PASSWORD
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
                    "password": STRONG_PASSWORD
                }))
                .expect("failed to serialize"),
            ))
            .expect("failed to build request");

        let resp = app
            .oneshot(second_req)
            .await
            .expect("second request failed");
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
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
                    "password": STRONG_PASSWORD
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
                    "password": STRONG_PASSWORD
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
