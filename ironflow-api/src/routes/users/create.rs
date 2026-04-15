//! `POST /api/v1/users` -- Create a new user (admin only).

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use validator::Validate;

use ironflow_auth::extractor::Authenticated;
use ironflow_auth::password;
use ironflow_store::entities::NewUser;
use ironflow_store::error::StoreError;

use crate::entities::{CreateUserRequest, UserResponse};
use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Create a new user account. Admin only.
///
/// # Errors
///
/// - 403 if the caller is not an admin
/// - 400 if input validation fails
/// - 409 if email or username is already taken
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        post,
        path = "/api/v1/users",
        tags = ["users"],
        request_body(content = CreateUserRequest, description = "User account details"),
        responses(
            (status = 201, description = "User created successfully", body = UserResponse),
            (status = 400, description = "Invalid input"),
            (status = 401, description = "Unauthorized"),
            (status = 403, description = "Forbidden (not an admin)"),
            (status = 409, description = "Email or username already taken")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn create_user(
    auth: Authenticated,
    State(state): State<AppState>,
    Json(req): Json<CreateUserRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if !auth.is_admin() {
        return Err(ApiError::Forbidden);
    }

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
            is_admin: Some(req.is_admin),
        })
        .await
        .map_err(|e| match e {
            StoreError::DuplicateEmail(_) => ApiError::DuplicateEmail,
            StoreError::DuplicateUsername(_) => ApiError::DuplicateUsername,
            other => ApiError::Store(other),
        })?;

    Ok((StatusCode::CREATED, ok(UserResponse::from(user))))
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
    use ironflow_engine::notify::Event;
    use ironflow_store::api_key_store::ApiKeyStore;
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::user_store::UserStore;
    use serde_json::{json, to_string};
    use std::sync::Arc;
    use tokio::sync::broadcast;
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
            secret: "test-secret-for-user-tests".to_string(),
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
        let (event_sender, _) = broadcast::channel::<Event>(1);
        AppState::new(
            store,
            user_store,
            api_key_store,
            Arc::new(engine),
            test_jwt_config(),
            "test-worker-token".to_string(),
            event_sender,
        )
    }

    fn make_auth_header(user_id: Uuid, is_admin: bool, state: &AppState) -> String {
        let token =
            AccessToken::for_user(user_id, "testuser", is_admin, &state.jwt_config).unwrap();
        format!("Bearer {}", token.0)
    }

    #[tokio::test]
    async fn create_user_as_admin() {
        let state = test_state();
        let admin_id = Uuid::now_v7();
        let auth_header = make_auth_header(admin_id, true, &state);
        let app = Router::new()
            .route("/", post(create_user))
            .with_state(state);

        let req = Request::builder()
            .uri("/")
            .method("POST")
            .header("content-type", "application/json")
            .header("authorization", auth_header)
            .body(Body::from(
                to_string(&json!({
                    "email": "new@example.com",
                    "username": "newuser",
                    "password": "password123",
                    "is_admin": false
                }))
                .unwrap(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    #[tokio::test]
    async fn create_user_as_member_forbidden() {
        let state = test_state();
        let member_id = Uuid::now_v7();
        let auth_header = make_auth_header(member_id, false, &state);
        let app = Router::new()
            .route("/", post(create_user))
            .with_state(state);

        let req = Request::builder()
            .uri("/")
            .method("POST")
            .header("content-type", "application/json")
            .header("authorization", auth_header)
            .body(Body::from(
                to_string(&json!({
                    "email": "new@example.com",
                    "username": "newuser",
                    "password": "password123",
                    "is_admin": false
                }))
                .unwrap(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn create_user_invalid_email() {
        let state = test_state();
        let admin_id = Uuid::now_v7();
        let auth_header = make_auth_header(admin_id, true, &state);
        let app = Router::new()
            .route("/", post(create_user))
            .with_state(state);

        let req = Request::builder()
            .uri("/")
            .method("POST")
            .header("content-type", "application/json")
            .header("authorization", auth_header)
            .body(Body::from(
                to_string(&json!({
                    "email": "invalid",
                    "username": "newuser",
                    "password": "password123",
                    "is_admin": false
                }))
                .unwrap(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
}
