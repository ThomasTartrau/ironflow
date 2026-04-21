//! `POST /api/v1/secrets` -- Create or update a secret (admin only).

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use validator::Validate;

use ironflow_auth::extractor::Authenticated;

use crate::entities::{SecretResponse, SetSecretRequest};
use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Create or update a secret. Admin only.
///
/// If the key already exists, the value is replaced. The response
/// never includes the secret value.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        post,
        path = "/api/v1/secrets",
        tags = ["secrets"],
        request_body(content = SetSecretRequest, description = "Secret key and value"),
        responses(
            (status = 201, description = "Secret created or updated", body = SecretResponse),
            (status = 400, description = "Invalid input"),
            (status = 401, description = "Unauthorized"),
            (status = 403, description = "Forbidden")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn create_secret(
    auth: Authenticated,
    State(state): State<AppState>,
    Json(req): Json<SetSecretRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if !auth.is_admin() {
        return Err(ApiError::Forbidden);
    }

    req.validate()
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    let secret = state.store.set_secret(&req.key, &req.value).await?;

    Ok((StatusCode::CREATED, ok(SecretResponse::from(secret))))
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::{get, put};
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
    use serde_json::{Value, json, to_string};
    use std::sync::Arc;
    use tokio::sync::broadcast;
    use tower::ServiceExt;
    use uuid::Uuid;

    use crate::routes::secrets;
    use crate::state::AppState;

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
            secret: "test-secret-for-secret-tests".to_string(),
            access_token_ttl_secs: 900,
            refresh_token_ttl_secs: 604800,
            cookie_domain: None,
            cookie_secure: false,
        })
    }

    fn test_state() -> AppState {
        let mut mem_store = InMemoryStore::new();
        mem_store.set_master_key(MasterKey::from_bytes(&[42u8; 32]).unwrap());
        let store: Arc<dyn Store> = Arc::new(mem_store);
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

    fn make_auth_header(is_admin: bool, state: &AppState) -> String {
        let user_id = Uuid::now_v7();
        let token =
            AccessToken::for_user(user_id, "testuser", is_admin, &state.jwt_config).unwrap();
        format!("Bearer {}", token.0)
    }

    fn secrets_router(state: AppState) -> Router {
        Router::new()
            .route(
                "/secrets",
                get(secrets::list::list_secrets).post(super::create_secret),
            )
            .route(
                "/secrets/{*key}",
                put(secrets::update::update_secret).delete(secrets::delete::delete_secret),
            )
            .with_state(state)
    }

    #[tokio::test]
    async fn create_secret_as_admin() {
        let state = test_state();
        let auth = make_auth_header(true, &state);
        let app = secrets_router(state);

        let req = Request::builder()
            .uri("/secrets")
            .method("POST")
            .header("content-type", "application/json")
            .header("authorization", &auth)
            .body(Body::from(
                to_string(&json!({"key": "demo/api-key", "value": "secret-value"})).unwrap(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["data"]["key"], "demo/api-key");
        assert!(json["data"]["value"].is_null() || json["data"].get("value").is_none());
    }

    #[tokio::test]
    async fn create_secret_as_member_forbidden() {
        let state = test_state();
        let auth = make_auth_header(false, &state);
        let app = secrets_router(state);

        let req = Request::builder()
            .uri("/secrets")
            .method("POST")
            .header("content-type", "application/json")
            .header("authorization", &auth)
            .body(Body::from(
                to_string(&json!({"key": "demo/key", "value": "val"})).unwrap(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn create_secret_invalid_key_chars() {
        let state = test_state();
        let auth = make_auth_header(true, &state);
        let app = secrets_router(state);

        let req = Request::builder()
            .uri("/secrets")
            .method("POST")
            .header("content-type", "application/json")
            .header("authorization", &auth)
            .body(Body::from(
                to_string(&json!({"key": "bad key with spaces!", "value": "val"})).unwrap(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn create_secret_invalid_key_leading_slash() {
        let state = test_state();
        let auth = make_auth_header(true, &state);
        let app = secrets_router(state);

        let req = Request::builder()
            .uri("/secrets")
            .method("POST")
            .header("content-type", "application/json")
            .header("authorization", &auth)
            .body(Body::from(
                to_string(&json!({"key": "/leading-slash", "value": "val"})).unwrap(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn list_secrets_empty() {
        let state = test_state();
        let auth = make_auth_header(true, &state);
        let app = secrets_router(state);

        let req = Request::builder()
            .uri("/secrets")
            .header("authorization", &auth)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["data"], json!([]));
    }

    #[tokio::test]
    async fn update_secret_not_found() {
        let state = test_state();
        let auth = make_auth_header(true, &state);
        let app = secrets_router(state);

        let req = Request::builder()
            .uri("/secrets/nonexistent/key")
            .method("PUT")
            .header("content-type", "application/json")
            .header("authorization", &auth)
            .body(Body::from(to_string(&json!({"value": "new-val"})).unwrap()))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn delete_secret_not_found() {
        let state = test_state();
        let auth = make_auth_header(true, &state);
        let app = secrets_router(state);

        let req = Request::builder()
            .uri("/secrets/nonexistent/key")
            .method("DELETE")
            .header("authorization", &auth)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn full_crud_lifecycle() {
        let state = test_state();
        let auth = make_auth_header(true, &state);

        let app = secrets_router(state.clone());
        let req = Request::builder()
            .uri("/secrets")
            .method("POST")
            .header("content-type", "application/json")
            .header("authorization", &auth)
            .body(Body::from(
                to_string(&json!({"key": "wf/inbox/token", "value": "v1"})).unwrap(),
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        let app = secrets_router(state.clone());
        let req = Request::builder()
            .uri("/secrets")
            .header("authorization", &auth)
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["data"].as_array().unwrap().len(), 1);
        assert_eq!(json["data"][0]["key"], "wf/inbox/token");

        let app = secrets_router(state.clone());
        let req = Request::builder()
            .uri("/secrets/wf/inbox/token")
            .method("PUT")
            .header("content-type", "application/json")
            .header("authorization", &auth)
            .body(Body::from(to_string(&json!({"value": "v2"})).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let app = secrets_router(state.clone());
        let req = Request::builder()
            .uri("/secrets/wf/inbox/token")
            .method("DELETE")
            .header("authorization", &auth)
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        let app = secrets_router(state.clone());
        let req = Request::builder()
            .uri("/secrets")
            .header("authorization", &auth)
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["data"].as_array().unwrap().len(), 0);
    }
}
