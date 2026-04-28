//! `POST /api/v1/auth/refresh` — Refresh access token using a refresh token.

use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::IntoResponse;

use ironflow_auth::cookies::{build_auth_cookie, build_refresh_cookie, extract_refresh_token};
use ironflow_auth::jwt::{AccessToken, RefreshToken};

use crate::error::ApiError;
use crate::state::AppState;

/// Refresh the access token using a valid refresh token from cookies.
///
/// # Errors
///
/// - 401 if the refresh token is missing, invalid, or expired
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        post,
        path = "/api/v1/auth/refresh",
        tags = ["auth"],
        responses(
            (status = 204, description = "Access token refreshed successfully, new cookies set"),
            (status = 401, description = "Invalid or expired refresh token")
        )
    )
)]
pub async fn refresh(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let raw_refresh = extract_refresh_token(&headers).ok_or(ApiError::Unauthorized)?;

    let claims = RefreshToken::decode(&raw_refresh, &state.jwt_config)
        .map_err(|_| ApiError::Unauthorized)?;

    let new_access = AccessToken::for_user(
        claims.user_id,
        &claims.username,
        claims.is_admin,
        &state.jwt_config,
    )
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    let new_refresh = RefreshToken::for_user(
        claims.user_id,
        &claims.username,
        claims.is_admin,
        &state.jwt_config,
    )
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    let mut response_headers = HeaderMap::new();
    if let Ok(val) = HeaderValue::from_str(&build_auth_cookie(&new_access.0, &state.jwt_config)) {
        response_headers.append("Set-Cookie", val);
    }
    if let Ok(val) = HeaderValue::from_str(&build_refresh_cookie(&new_refresh.0, &state.jwt_config))
    {
        response_headers.append("Set-Cookie", val);
    }

    Ok((StatusCode::NO_CONTENT, response_headers))
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::post;
    use ironflow_auth::jwt::{JwtConfig, RefreshToken};
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::context::WorkflowContext;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
    use ironflow_engine::notify::Event;
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
            secret: "test-secret-for-auth-tests".to_string(),
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

    #[tokio::test]
    async fn refresh_success() {
        let state = test_state();
        let jwt_config = test_jwt_config();
        let user_id = Uuid::now_v7();

        let refresh_token = RefreshToken::for_user(user_id, "testuser", false, &jwt_config)
            .expect("failed to create refresh token");

        let app = Router::new().route("/", post(refresh)).with_state(state);

        let req = Request::builder()
            .uri("/")
            .method("POST")
            .header("content-type", "application/json")
            .header("Cookie", format!("ironflow_refresh={}", refresh_token.0))
            .body(Body::empty())
            .expect("failed to build request");

        let resp = app.oneshot(req).await.expect("request failed");
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        let set_cookie = resp.headers().get_all("set-cookie");
        assert!(set_cookie.iter().count() > 0);
    }

    #[tokio::test]
    async fn refresh_missing_token() {
        let state = test_state();
        let app = Router::new().route("/", post(refresh)).with_state(state);

        let req = Request::builder()
            .uri("/")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::empty())
            .expect("failed to build request");

        let resp = app.oneshot(req).await.expect("request failed");
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn refresh_invalid_token() {
        let state = test_state();
        let app = Router::new().route("/", post(refresh)).with_state(state);

        let req = Request::builder()
            .uri("/")
            .method("POST")
            .header("content-type", "application/json")
            .header("Cookie", "refresh_token=invalid-token-data")
            .body(Body::empty())
            .expect("failed to build request");

        let resp = app.oneshot(req).await.expect("request failed");
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}
