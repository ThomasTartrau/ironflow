//! `GET /api/v1/secrets` -- List secrets (admin only, never exposes values).

use axum::extract::{Query, State};
use axum::response::IntoResponse;
use serde::Deserialize;

use ironflow_auth::extractor::Authenticated;

use crate::entities::SecretResponse;
use crate::error::ApiError;
use crate::response::ok_paged;
use crate::state::AppState;

/// Query parameters for listing secrets.
#[cfg_attr(feature = "openapi", derive(utoipa::IntoParams))]
#[derive(Debug, Deserialize)]
pub struct ListSecretsQuery {
    /// Filter by key prefix (e.g. `workflows/inbox/`).
    pub prefix: Option<String>,
    /// Page number (1-based, default 1).
    pub page: Option<u32>,
    /// Items per page (default 50, max 100).
    pub per_page: Option<u32>,
}

/// List secret metadata. Admin only.
///
/// Returns key, id, and timestamps. Never returns encrypted or decrypted values.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/secrets",
        tags = ["secrets"],
        params(ListSecretsQuery),
        responses(
            (status = 200, description = "Secrets listed", body = Vec<SecretResponse>),
            (status = 401, description = "Unauthorized"),
            (status = 403, description = "Forbidden")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn list_secrets(
    auth: Authenticated,
    State(state): State<AppState>,
    Query(query): Query<ListSecretsQuery>,
) -> Result<impl IntoResponse, ApiError> {
    if !auth.is_admin() {
        return Err(ApiError::Forbidden);
    }

    let prefix = query.prefix.as_deref().unwrap_or("");
    let page = query.page.unwrap_or(1).max(1);
    let per_page = query.per_page.unwrap_or(50).clamp(1, 100);

    let result = state.store.list_secrets(prefix, page, per_page).await?;

    let data: Vec<SecretResponse> = result.items.into_iter().map(SecretResponse::from).collect();

    Ok(ok_paged(data, result.page, result.per_page, result.total))
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
    async fn list_secrets_admin_only() {
        let state = test_state();
        let auth_header = make_regular_token(&state);

        let app = Router::new()
            .route("/", get(list_secrets))
            .with_state(state);

        let req = Request::builder()
            .uri("/")
            .method("GET")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn list_secrets_admin_succeeds() {
        let state = test_state();
        let auth_header = make_admin_token(&state);

        let app = Router::new()
            .route("/", get(list_secrets))
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
        assert!(json_val["data"].is_array());
        assert!(json_val["meta"].is_object());
    }

    #[tokio::test]
    async fn list_secrets_includes_pagination() {
        let state = test_state();
        let auth_header = make_admin_token(&state);

        let app = Router::new()
            .route("/", get(list_secrets))
            .with_state(state);

        let req = Request::builder()
            .uri("/?page=1&per_page=10")
            .method("GET")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = serde_json::from_slice(&body).unwrap();
        assert_eq!(json_val["meta"]["page"], 1);
        assert_eq!(json_val["meta"]["per_page"], 10);
    }

    #[tokio::test]
    async fn list_secrets_clamps_per_page_max() {
        let state = test_state();
        let auth_header = make_admin_token(&state);

        let app = Router::new()
            .route("/", get(list_secrets))
            .with_state(state);

        let req = Request::builder()
            .uri("/?per_page=500")
            .method("GET")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = serde_json::from_slice(&body).unwrap();
        assert_eq!(json_val["meta"]["per_page"], 100);
    }
}
