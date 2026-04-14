//! `DELETE /api/v1/users/{id}` -- Delete a user (admin only).

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use uuid::Uuid;

use ironflow_auth::extractor::Authenticated;
use ironflow_store::error::StoreError;

use crate::error::ApiError;
use crate::state::AppState;

/// Delete a user account. Admin only.
///
/// # Errors
///
/// - 403 if the caller is not an admin
/// - 400 if trying to delete self
/// - 404 if the user does not exist
pub async fn delete_user(
    auth: Authenticated,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    if !auth.is_admin() {
        return Err(ApiError::Forbidden);
    }

    if auth.user_id == id {
        return Err(ApiError::BadRequest(
            "cannot delete your own account".to_string(),
        ));
    }

    state
        .user_store
        .delete_user(id)
        .await
        .map_err(|e| match e {
            StoreError::UserNotFound(id) => ApiError::UserNotFound(id),
            other => ApiError::Store(other),
        })?;

    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::delete;
    use ironflow_auth::jwt::{AccessToken, JwtConfig};
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::context::WorkflowContext;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
    use ironflow_store::api_key_store::ApiKeyStore;
    use ironflow_store::entities::NewUser;
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
        AppState::new(
            store,
            user_store,
            api_key_store,
            Arc::new(engine),
            test_jwt_config(),
            "test-worker-token".to_string(),
        )
    }

    fn make_auth_header(user_id: Uuid, is_admin: bool, state: &AppState) -> String {
        let token =
            AccessToken::for_user(user_id, "testuser", is_admin, &state.jwt_config).unwrap();
        format!("Bearer {}", token.0)
    }

    #[tokio::test]
    async fn delete_user_as_admin() {
        let state = test_state();
        let admin = state
            .user_store
            .create_user(NewUser {
                email: "admin@example.com".to_string(),
                username: "admin".to_string(),
                password_hash: "hash".to_string(),
                is_admin: None,
            })
            .await
            .unwrap();

        let target = state
            .user_store
            .create_user(NewUser {
                email: "target@example.com".to_string(),
                username: "target".to_string(),
                password_hash: "hash".to_string(),
                is_admin: None,
            })
            .await
            .unwrap();

        let auth_header = make_auth_header(admin.id, true, &state);
        let app = Router::new()
            .route("/{id}", delete(delete_user))
            .with_state(state);

        let req = Request::builder()
            .uri(format!("/{}", target.id))
            .method("DELETE")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn delete_user_self_forbidden() {
        let state = test_state();
        let admin = state
            .user_store
            .create_user(NewUser {
                email: "admin@example.com".to_string(),
                username: "admin".to_string(),
                password_hash: "hash".to_string(),
                is_admin: None,
            })
            .await
            .unwrap();

        let auth_header = make_auth_header(admin.id, true, &state);
        let app = Router::new()
            .route("/{id}", delete(delete_user))
            .with_state(state);

        let req = Request::builder()
            .uri(format!("/{}", admin.id))
            .method("DELETE")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn delete_user_as_member_forbidden() {
        let state = test_state();
        let member_id = Uuid::now_v7();
        let target_id = Uuid::now_v7();
        let auth_header = make_auth_header(member_id, false, &state);
        let app = Router::new()
            .route("/{id}", delete(delete_user))
            .with_state(state);

        let req = Request::builder()
            .uri(format!("/{target_id}"))
            .method("DELETE")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }
}
