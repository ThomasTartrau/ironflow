//! `DELETE /api/v1/secrets/:key` -- Delete a secret (admin only).

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;

use ironflow_auth::extractor::Authenticated;

use crate::error::ApiError;
use crate::state::AppState;

/// Delete a secret by key. Admin only.
///
/// # Errors
///
/// - 403 if the caller is not an admin
/// - 404 if the secret key does not exist
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        delete,
        path = "/api/v1/secrets/{key}",
        tags = ["secrets"],
        params(("key" = String, Path, description = "Secret key")),
        responses(
            (status = 204, description = "Secret deleted"),
            (status = 401, description = "Unauthorized"),
            (status = 403, description = "Forbidden"),
            (status = 404, description = "Secret not found")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn delete_secret(
    auth: Authenticated,
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    if !auth.is_admin() {
        return Err(ApiError::Forbidden);
    }

    let deleted = state.store.delete_secret(&key).await?;
    if !deleted {
        return Err(ApiError::SecretNotFound(key));
    }

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
    use ironflow_store::crypto::MasterKey;
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
        let mut in_mem_store = InMemoryStore::new();
        let master_key = MasterKey::from_bytes(&[42u8; 32]).unwrap();
        in_mem_store.set_master_key(master_key);
        let store: Arc<dyn Store> = Arc::new(in_mem_store);
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

    fn make_admin_token(state: &AppState) -> String {
        let user_id = Uuid::now_v7();
        let token = AccessToken::for_user(user_id, "admin", true, &state.jwt_config).unwrap();
        format!("Bearer {}", token.0)
    }

    fn make_regular_token(state: &AppState) -> String {
        let user_id = Uuid::now_v7();
        let token = AccessToken::for_user(user_id, "user", false, &state.jwt_config).unwrap();
        format!("Bearer {}", token.0)
    }

    #[tokio::test]
    async fn delete_secret_admin_only() {
        let state = test_state();
        let auth_header = make_regular_token(&state);

        let app = Router::new()
            .route("/{key}", delete(delete_secret))
            .with_state(state);

        let req = Request::builder()
            .uri("/my-secret")
            .method("DELETE")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn delete_secret_success() {
        let state = test_state();
        state
            .store
            .set_secret("api-key", "secret-value")
            .await
            .unwrap();

        let auth_header = make_admin_token(&state);

        let app = Router::new()
            .route("/{key}", delete(delete_secret))
            .with_state(state);

        let req = Request::builder()
            .uri("/api-key")
            .method("DELETE")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn delete_secret_not_found() {
        let state = test_state();
        let auth_header = make_admin_token(&state);

        let app = Router::new()
            .route("/{key}", delete(delete_secret))
            .with_state(state);

        let req = Request::builder()
            .uri("/nonexistent-secret")
            .method("DELETE")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
