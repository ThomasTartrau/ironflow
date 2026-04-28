//! `PUT /api/v1/secrets/:key` -- Update a secret value (admin only).

use axum::Json;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use serde::Deserialize;
use validator::Validate;

use ironflow_auth::extractor::Authenticated;

use crate::entities::SecretResponse;
use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Request body for updating a secret value.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Deserialize, Validate)]
pub struct UpdateSecretRequest {
    /// New secret value (plaintext, will be encrypted at rest).
    #[validate(length(min = 1, max = 65536))]
    pub value: String,
}

/// Update a secret's value. Admin only.
///
/// The key is taken from the URL path. The response never includes
/// the secret value.
///
/// # Errors
///
/// - 403 if the caller is not an admin
/// - 404 if the secret key does not exist
/// - 400 if the value is invalid
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        put,
        path = "/api/v1/secrets/{key}",
        tags = ["secrets"],
        params(("key" = String, Path, description = "Secret key")),
        request_body(content = UpdateSecretRequest, description = "New secret value"),
        responses(
            (status = 200, description = "Secret updated", body = SecretResponse),
            (status = 400, description = "Invalid input"),
            (status = 401, description = "Unauthorized"),
            (status = 403, description = "Forbidden"),
            (status = 404, description = "Secret not found")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn update_secret(
    auth: Authenticated,
    State(state): State<AppState>,
    Path(key): Path<String>,
    Json(req): Json<UpdateSecretRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if !auth.is_admin() {
        return Err(ApiError::Forbidden);
    }

    req.validate()
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    let existing = state.store.get_secret(&key).await?;
    if existing.is_none() {
        return Err(ApiError::SecretNotFound(key));
    }

    let secret = state.store.set_secret(&key, &req.value).await?;

    Ok(ok(SecretResponse::from(secret)))
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::put;
    use http_body_util::BodyExt;
    use ironflow_auth::jwt::{AccessToken, JwtConfig};
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::context::WorkflowContext;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
    use ironflow_engine::notify::Event;
    use ironflow_store::crypto::MasterKey;
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::store::Store;
    use serde_json::{Value as JsonValue, json};
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
    async fn update_secret_admin_only() {
        let state = test_state();
        state
            .store
            .set_secret("api-key", "old-value")
            .await
            .unwrap();

        let auth_header = make_regular_token(&state);

        let app = Router::new()
            .route("/{key}", put(update_secret))
            .with_state(state);

        let body = json!({ "value": "new-value" });
        let req = Request::builder()
            .uri("/api-key")
            .method("PUT")
            .header("authorization", auth_header)
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_string(&body).unwrap()))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn update_secret_success() {
        let state = test_state();
        state
            .store
            .set_secret("api-key", "old-value")
            .await
            .unwrap();

        let auth_header = make_admin_token(&state);

        let app = Router::new()
            .route("/{key}", put(update_secret))
            .with_state(state);

        let body = json!({ "value": "new-value" });
        let req = Request::builder()
            .uri("/api-key")
            .method("PUT")
            .header("authorization", auth_header)
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_string(&body).unwrap()))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let resp_body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = serde_json::from_slice(&resp_body).unwrap();
        assert_eq!(json_val["data"]["key"], "api-key");
    }

    #[tokio::test]
    async fn update_secret_not_found() {
        let state = test_state();
        let auth_header = make_admin_token(&state);

        let app = Router::new()
            .route("/{key}", put(update_secret))
            .with_state(state);

        let body = json!({ "value": "new-value" });
        let req = Request::builder()
            .uri("/nonexistent-secret")
            .method("PUT")
            .header("authorization", auth_header)
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_string(&body).unwrap()))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn update_secret_empty_value_validation() {
        let state = test_state();
        state.store.set_secret("api-key", "value").await.ok();

        let auth_header = make_admin_token(&state);

        let app = Router::new()
            .route("/{key}", put(update_secret))
            .with_state(state);

        let body = json!({ "value": "" });
        let req = Request::builder()
            .uri("/api-key")
            .method("PUT")
            .header("authorization", auth_header)
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_string(&body).unwrap()))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
}
