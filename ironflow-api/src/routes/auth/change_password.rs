//! `PATCH /api/v1/auth/password` -- Change the current user's password.

use axum::Json;
use axum::extract::State;
use axum::response::IntoResponse;
use validator::Validate;

use ironflow_auth::extractor::AuthenticatedUser;
use ironflow_auth::password;

use crate::entities::ChangePasswordRequest;
use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Change the authenticated user's password.
///
/// Verifies the old password, hashes the new one, and persists it.
///
/// # Errors
///
/// - 400 if old password is incorrect or new password fails validation
/// - 401 if no valid token is provided
/// - 404 if the user no longer exists in the store
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        patch,
        path = "/api/v1/auth/password",
        tags = ["auth"],
        request_body(content = ChangePasswordRequest, description = "Old and new passwords"),
        responses(
            (status = 200, description = "Password changed successfully"),
            (status = 400, description = "Invalid old password or validation error"),
            (status = 401, description = "Unauthorized")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn change_password(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Json(req): Json<ChangePasswordRequest>,
) -> Result<impl IntoResponse, ApiError> {
    req.validate()
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    let stored_user = state
        .store
        .find_user_by_id(user.user_id)
        .await?
        .ok_or(ApiError::Unauthorized)?;

    let valid = password::verify(&req.old_password, &stored_user.password_hash)
        .map_err(|_| ApiError::Internal("password verification failed".to_string()))?;

    if !valid {
        return Err(ApiError::BadRequest("incorrect old password".to_string()));
    }

    let new_hash = password::hash(&req.new_password)
        .map_err(|_| ApiError::Internal("password hashing failed".to_string()))?;

    state
        .store
        .update_user_password(user.user_id, new_hash)
        .await?;

    Ok(ok(serde_json::json!({ "message": "password changed" })))
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::patch;
    use http_body_util::BodyExt;
    use ironflow_auth::jwt::{AccessToken, JwtConfig};
    use ironflow_auth::password;
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::context::WorkflowContext;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
    use ironflow_engine::notify::Event;
    use ironflow_store::entities::NewUser;
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::store::Store;
    use serde_json::json;
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
            secret: "test-secret-for-password-tests".to_string(),
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

    fn make_auth_header(user_id: Uuid, state: &AppState) -> String {
        let token = AccessToken::for_user(user_id, "testuser", false, &state.jwt_config)
            .expect("failed to create token");
        format!("Bearer {}", token.0)
    }

    #[tokio::test]
    async fn change_password_success() {
        let state = test_state();
        let hash = password::hash("oldpassword123").expect("hash");
        let user = state
            .store
            .create_user(NewUser {
                email: "test@example.com".to_string(),
                username: "testuser".to_string(),
                password_hash: hash,
                is_admin: None,
            })
            .await
            .expect("create user");

        let auth_header = make_auth_header(user.id, &state);
        let app = Router::new()
            .route("/", patch(change_password))
            .with_state(state.clone());

        let req = Request::builder()
            .uri("/")
            .method("PATCH")
            .header("authorization", &auth_header)
            .header("content-type", "application/json")
            .body(Body::from(
                json!({"old_password": "oldpassword123", "new_password": "newpassword456"})
                    .to_string(),
            ))
            .expect("build request");

        let resp = app.oneshot(req).await.expect("request");
        assert_eq!(resp.status(), StatusCode::OK);

        let stored = state
            .store
            .find_user_by_id(user.id)
            .await
            .expect("find")
            .expect("some");
        assert!(password::verify("newpassword456", &stored.password_hash).expect("verify"));
    }

    #[tokio::test]
    async fn change_password_wrong_old() {
        let state = test_state();
        let hash = password::hash("oldpassword123").expect("hash");
        let user = state
            .store
            .create_user(NewUser {
                email: "test@example.com".to_string(),
                username: "testuser".to_string(),
                password_hash: hash,
                is_admin: None,
            })
            .await
            .expect("create user");

        let auth_header = make_auth_header(user.id, &state);
        let app = Router::new()
            .route("/", patch(change_password))
            .with_state(state);

        let req = Request::builder()
            .uri("/")
            .method("PATCH")
            .header("authorization", &auth_header)
            .header("content-type", "application/json")
            .body(Body::from(
                json!({"old_password": "wrongpassword", "new_password": "newpassword456"})
                    .to_string(),
            ))
            .expect("build request");

        let resp = app.oneshot(req).await.expect("request");
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let body = resp
            .into_body()
            .collect()
            .await
            .expect("collect")
            .to_bytes();
        let json_val: serde_json::Value = serde_json::from_slice(&body).expect("parse");
        assert!(
            json_val["error"]["message"]
                .as_str()
                .expect("msg")
                .contains("incorrect old password")
        );
    }

    #[tokio::test]
    async fn change_password_without_auth() {
        let state = test_state();
        let app = Router::new()
            .route("/", patch(change_password))
            .with_state(state);

        let req = Request::builder()
            .uri("/")
            .method("PATCH")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({"old_password": "old", "new_password": "newpassword456"}).to_string(),
            ))
            .expect("build request");

        let resp = app.oneshot(req).await.expect("request");
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}
