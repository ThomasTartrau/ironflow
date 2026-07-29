//! `GET /api/v1/secrets/key-versions` -- Encryption key ring status (admin only).

use axum::extract::State;
use axum::response::IntoResponse;

use ironflow_auth::extractor::Authenticated;

use crate::entities::KeyVersionsResponse;
use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Report how the configured key ring lines up with the stored secrets.
///
/// This is what tells an operator whether an old key can be dropped from
/// `IRONFLOW_SECRET_KEYS` without breaking the next startup: a version listed
/// in `retirable` is configured, not active, and used by no secret.
///
/// A non-empty `missing` means some stored secrets cannot be decrypted with
/// the current configuration -- the server refuses to start in that state.
///
/// # Errors
///
/// - 401 if the caller is not authenticated
/// - 403 if the caller is not an admin
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/secrets/key-versions",
        tags = ["secrets"],
        responses(
            (status = 200, description = "Key ring status", body = KeyVersionsResponse),
            (status = 401, description = "Unauthorized"),
            (status = 403, description = "Forbidden")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn secret_key_versions(
    auth: Authenticated,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ApiError> {
    if !auth.is_admin() {
        return Err(ApiError::Forbidden);
    }

    let status = state.store.secret_key_status().await?;

    Ok(ok(KeyVersionsResponse::from(status)))
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
    use ironflow_store::crypto::KeyRing;
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::store::Store;
    use serde_json::Value;
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

    fn hex_key(byte: u8) -> String {
        format!("{byte:02x}").repeat(32)
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

    fn test_state(active: i32) -> AppState {
        let spec = format!("1:{},2:{}", hex_key(0xaa), hex_key(0xbb));
        let mut in_mem_store = InMemoryStore::new();
        in_mem_store.set_key_ring(KeyRing::from_spec(&spec, Some(active)).unwrap());

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

    fn app(state: AppState) -> Router {
        Router::new()
            .route("/key-versions", get(secret_key_versions))
            .with_state(state)
    }

    fn request(auth: Option<&str>) -> Request<Body> {
        let mut builder = Request::builder().uri("/key-versions").method("GET");
        if let Some(auth) = auth {
            builder = builder.header("authorization", auth);
        }
        builder.body(Body::empty()).unwrap()
    }

    #[tokio::test]
    async fn key_versions_requires_authentication() {
        let resp = app(test_state(2)).oneshot(request(None)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn key_versions_is_admin_only() {
        let state = test_state(2);
        let token =
            AccessToken::for_user(Uuid::now_v7(), "user", false, &state.jwt_config).unwrap();
        let auth = format!("Bearer {}", token.0);

        let resp = app(state).oneshot(request(Some(&auth))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn key_versions_reports_a_retirable_version() {
        let state = test_state(2);
        state.store.set_secret("a", "va").await.unwrap();

        let token =
            AccessToken::for_user(Uuid::now_v7(), "admin", true, &state.jwt_config).unwrap();
        let auth = format!("Bearer {}", token.0);

        let resp = app(state).oneshot(request(Some(&auth))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let body: Value = serde_json::from_slice(&bytes).unwrap();

        assert_eq!(body["data"]["active"], 2);
        assert_eq!(body["data"]["configured"], serde_json::json!([1, 2]));
        assert_eq!(body["data"]["in_use"], serde_json::json!([2]));
        assert_eq!(body["data"]["missing"], serde_json::json!([]));
        assert_eq!(body["data"]["retirable"], serde_json::json!([1]));
    }

    #[tokio::test]
    async fn key_versions_never_marks_the_active_version_retirable() {
        let state = test_state(1);
        state.store.set_secret("a", "va").await.unwrap();

        let token =
            AccessToken::for_user(Uuid::now_v7(), "admin", true, &state.jwt_config).unwrap();
        let auth = format!("Bearer {}", token.0);

        let resp = app(state).oneshot(request(Some(&auth))).await.unwrap();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let body: Value = serde_json::from_slice(&bytes).unwrap();

        assert_eq!(body["data"]["in_use"], serde_json::json!([1]));
        assert_eq!(body["data"]["retirable"], serde_json::json!([2]));
    }
}
