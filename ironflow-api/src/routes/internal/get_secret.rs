//! `GET /api/v1/internal/secrets/:key` -- Get a secret by key (worker only).

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use ironflow_store::entities::Secret;

use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Internal secret response that INCLUDES the decrypted value.
///
/// Unlike the public `SecretResponse`, this is only sent to the worker
/// over the internal API (protected by WORKER_TOKEN).
#[derive(Serialize)]
struct InternalSecretResponse {
    id: Uuid,
    key: String,
    value: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<Secret> for InternalSecretResponse {
    fn from(s: Secret) -> Self {
        Self {
            id: s.id,
            key: s.key,
            value: s.value,
            created_at: s.created_at,
            updated_at: s.updated_at,
        }
    }
}

/// Get a decrypted secret by key. Worker-only (WORKER_TOKEN auth).
pub async fn get_secret(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let secret = state
        .store
        .get_secret(&key)
        .await?
        .ok_or(ApiError::SecretNotFound(key))?;

    Ok(ok(InternalSecretResponse::from(secret)))
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use http_body_util::BodyExt;
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::notify::Event;
    use ironflow_store::crypto::MasterKey;
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::store::Store;
    use serde_json::Value as JsonValue;
    use std::sync::Arc;
    use tokio::sync::broadcast;
    use tower::ServiceExt;

    use super::*;

    fn test_state() -> AppState {
        let mut in_mem_store = InMemoryStore::new();
        let master_key = MasterKey::from_bytes(&[42u8; 32]).unwrap();
        in_mem_store.set_master_key(master_key);
        let store: Arc<dyn Store> = Arc::new(in_mem_store);
        let provider = Arc::new(ClaudeCodeProvider::new());
        let engine = Arc::new(Engine::new(store.clone(), provider));
        let jwt_config = Arc::new(ironflow_auth::jwt::JwtConfig {
            secret: "test-secret".to_string(),
            access_token_ttl_secs: 900,
            refresh_token_ttl_secs: 604800,
            cookie_domain: None,
            cookie_secure: false,
        });
        let (event_sender, _) = broadcast::channel::<Event>(1);
        AppState::new(
            store,
            engine,
            jwt_config,
            "test-worker-token".to_string(),
            event_sender,
        )
    }

    #[tokio::test]
    async fn get_secret_returns_decrypted_value() {
        let state = test_state();
        state
            .store
            .set_secret("db-password", "super-secret")
            .await
            .unwrap();

        let app = Router::new()
            .route("/{key}", get(get_secret))
            .with_state(state);

        let req = Request::builder()
            .uri("/db-password")
            .method("GET")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = serde_json::from_slice(&body).unwrap();
        assert_eq!(json_val["data"]["key"], "db-password");
        assert_eq!(json_val["data"]["value"], "super-secret");
    }

    #[tokio::test]
    async fn get_secret_not_found() {
        let state = test_state();

        let app = Router::new()
            .route("/{key}", get(get_secret))
            .with_state(state);

        let req = Request::builder()
            .uri("/nonexistent")
            .method("GET")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
