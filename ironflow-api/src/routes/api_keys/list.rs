//! `GET /api/v1/api-keys` -- List API keys for the authenticated user.

use axum::extract::State;
use axum::response::IntoResponse;
use chrono::{DateTime, Utc};
use ironflow_auth::extractor::AuthenticatedUser;
use ironflow_store::entities::ApiKeyScope;
use serde::Serialize;
use uuid::Uuid;

use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// API key summary (never includes the hash or raw key).
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Serialize)]
pub struct ApiKeyResponse {
    /// API key ID.
    pub id: Uuid,
    /// Key name.
    pub name: String,
    /// First characters for identification.
    pub key_prefix: String,
    /// Granted scopes.
    pub scopes: Vec<ApiKeyScope>,
    /// Whether the key is active.
    pub is_active: bool,
    /// Expiration date.
    pub expires_at: Option<DateTime<Utc>>,
    /// Last used date.
    pub last_used_at: Option<DateTime<Utc>>,
    /// Creation date.
    pub created_at: DateTime<Utc>,
    /// Per-key rate limit override (requests per minute), if set.
    pub rate_limit_override: Option<u32>,
}

/// List all API keys for the authenticated user.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/api-keys",
        tags = ["api-keys"],
        responses(
            (status = 200, description = "List of API keys", body = Vec<ApiKeyResponse>),
            (status = 401, description = "Unauthorized")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn list_api_keys(
    user: AuthenticatedUser,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ApiError> {
    let keys = state
        .store
        .list_api_keys_by_user(user.user_id)
        .await
        .map_err(ApiError::from)?;

    let responses: Vec<ApiKeyResponse> = keys
        .into_iter()
        .map(|k| ApiKeyResponse {
            id: k.id,
            name: k.name,
            key_prefix: k.key_prefix,
            scopes: k.scopes,
            is_active: k.is_active,
            expires_at: k.expires_at,
            last_used_at: k.last_used_at,
            created_at: k.created_at,
            rate_limit_override: k.rate_limit_override,
        })
        .collect();

    Ok(ok(responses))
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use http_body_util::BodyExt;
    use ironflow_auth::jwt::{AccessToken, JwtConfig};
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::context::WorkflowContext;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
    use ironflow_engine::notify::Event;
    use ironflow_store::entities::{NewApiKey, NewUser};
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::store::Store;
    use serde_json::Value as JsonValue;
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

    #[tokio::test]
    async fn list_api_keys_empty() {
        let state = test_state();
        let user_id = Uuid::now_v7();
        let token = AccessToken::for_user(user_id, "testuser", false, &state.jwt_config).unwrap();
        let auth_header = format!("Bearer {}", token.0);

        let app = Router::new()
            .route("/", get(list_api_keys))
            .with_state(state);

        let req = Request::builder()
            .uri("/")
            .method("GET")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = serde_json::from_slice(&body).unwrap();
        assert_eq!(json_val["data"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn list_api_keys_returns_user_keys() {
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

        let _key1 = state
            .store
            .create_api_key(NewApiKey {
                user_id: user.id,
                name: "key1".to_string(),
                key_hash: "hash".to_string(),
                key_prefix: "sk_".to_string(),
                scopes: vec![],
                expires_at: None,
                rate_limit_override: None,
            })
            .await
            .unwrap();

        let _key2 = state
            .store
            .create_api_key(NewApiKey {
                user_id: user.id,
                name: "key2".to_string(),
                key_hash: "hash".to_string(),
                key_prefix: "sk_".to_string(),
                scopes: vec![],
                expires_at: None,
                rate_limit_override: None,
            })
            .await
            .unwrap();

        let token = AccessToken::for_user(user.id, "testuser", false, &state.jwt_config).unwrap();
        let auth_header = format!("Bearer {}", token.0);

        let app = Router::new()
            .route("/", get(list_api_keys))
            .with_state(state);

        let req = Request::builder()
            .uri("/")
            .method("GET")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = serde_json::from_slice(&body).unwrap();
        assert_eq!(json_val["data"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn list_api_keys_does_not_leak_other_users_keys() {
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

        state
            .store
            .create_api_key(NewApiKey {
                user_id: user1.id,
                name: "user1-key".to_string(),
                key_hash: "hash".to_string(),
                key_prefix: "sk_".to_string(),
                scopes: vec![],
                expires_at: None,
                rate_limit_override: None,
            })
            .await
            .unwrap();

        state
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

        let token = AccessToken::for_user(user1.id, "user1", false, &state.jwt_config).unwrap();
        let auth_header = format!("Bearer {}", token.0);

        let app = Router::new()
            .route("/", get(list_api_keys))
            .with_state(state);

        let req = Request::builder()
            .uri("/")
            .method("GET")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = serde_json::from_slice(&body).unwrap();
        let keys = json_val["data"].as_array().unwrap();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0]["name"], "user1-key");
    }
}
