//! `DELETE /api/v1/api-keys/:id` -- Delete an API key.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use ironflow_auth::extractor::AuthenticatedUser;
use uuid::Uuid;

use crate::error::ApiError;
use crate::state::AppState;

/// Delete an API key owned by the authenticated user.
///
/// # Errors
///
/// - 404 if the key does not exist or belongs to another user
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        delete,
        path = "/api/v1/api-keys/{id}",
        tags = ["api-keys"],
        params(
            ("id" = Uuid, Path, description = "API key ID")
        ),
        responses(
            (status = 204, description = "API key deleted successfully"),
            (status = 401, description = "Unauthorized"),
            (status = 404, description = "API key not found")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn delete_api_key(
    user: AuthenticatedUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let key = state
        .store
        .find_api_key_by_id(id)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::ApiKeyNotFound(id))?;

    if key.user_id != user.user_id {
        return Err(ApiError::ApiKeyNotFound(id));
    }

    state
        .store
        .delete_api_key(id)
        .await
        .map_err(ApiError::from)?;

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
    use ironflow_engine::notify::Event;
    use ironflow_store::entities::{NewApiKey, NewUser};
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::store::Store;
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
            secret: "test-secret".to_string(),
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
        engine.register(TestWorkflow).unwrap();
        let (event_sender, _) = broadcast::channel::<Event>(1);
        AppState::new(
            store,
            Arc::new(engine),
            test_jwt_config(),
            "test-worker-token".to_string(),
            event_sender,
        )
    }

    fn make_auth_header(state: &AppState) -> String {
        let user_id = Uuid::now_v7();
        let token = AccessToken::for_user(user_id, "testuser", false, &state.jwt_config).unwrap();
        format!("Bearer {}", token.0)
    }

    #[tokio::test]
    async fn delete_api_key_success() {
        let state = test_state();
        let user = state
            .store
            .create_user(NewUser {
                email: "test@example.com".to_string(),
                username: "testuser".to_string(),
                password_hash: "hash".to_string(),
                is_admin: None,
            })
            .await
            .unwrap();

        let key = state
            .store
            .create_api_key(NewApiKey {
                user_id: user.id,
                name: "test-key".to_string(),
                key_hash: "hash".to_string(),
                key_prefix: "sk_".to_string(),
                scopes: vec![],
                expires_at: None,
                rate_limit_override: None,
            })
            .await
            .unwrap();

        let auth_header = {
            let token =
                AccessToken::for_user(user.id, "testuser", false, &state.jwt_config).unwrap();
            format!("Bearer {}", token.0)
        };

        let app = Router::new()
            .route("/{id}", delete(delete_api_key))
            .with_state(state);

        let req = Request::builder()
            .uri(format!("/{}", key.id))
            .method("DELETE")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn delete_api_key_not_found() {
        let state = test_state();
        let auth_header = make_auth_header(&state);

        let app = Router::new()
            .route("/{id}", delete(delete_api_key))
            .with_state(state);

        let key_id = Uuid::now_v7();
        let req = Request::builder()
            .uri(format!("/{}", key_id))
            .method("DELETE")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn delete_api_key_not_owned_by_user() {
        let state = test_state();

        let user1 = state
            .store
            .create_user(NewUser {
                email: "user1@example.com".to_string(),
                username: "user1".to_string(),
                password_hash: "hash".to_string(),
                is_admin: None,
            })
            .await
            .unwrap();

        let user2 = state
            .store
            .create_user(NewUser {
                email: "user2@example.com".to_string(),
                username: "user2".to_string(),
                password_hash: "hash".to_string(),
                is_admin: None,
            })
            .await
            .unwrap();

        let key = state
            .store
            .create_api_key(NewApiKey {
                user_id: user2.id,
                name: "user2-key".to_string(),
                key_hash: "hash".to_string(),
                key_prefix: "sk_".to_string(),
                scopes: vec![],
                expires_at: None,
                rate_limit_override: None,
            })
            .await
            .unwrap();

        let auth_header = {
            let token = AccessToken::for_user(user1.id, "user1", false, &state.jwt_config).unwrap();
            format!("Bearer {}", token.0)
        };

        let app = Router::new()
            .route("/{id}", delete(delete_api_key))
            .with_state(state);

        let req = Request::builder()
            .uri(format!("/{}", key.id))
            .method("DELETE")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
