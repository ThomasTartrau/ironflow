//! `POST /api/v1/auth/refresh` — Refresh access token using a refresh token.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;

use ironflow_auth::cookies::extract_refresh_token;
use ironflow_auth::jwt::{RefreshToken, token_hash};

use crate::error::ApiError;
use crate::routes::auth::session::issue_session;
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

    // A refresh token is single use: consuming it deletes the stored hash, so
    // a replayed token, or one revoked by sign-out, finds nothing.
    let owner = state
        .store
        .consume_refresh_token(&token_hash(&raw_refresh))
        .await?
        .ok_or(ApiError::Unauthorized)?;
    if owner != claims.user_id {
        return Err(ApiError::Unauthorized);
    }

    // The new pair is minted from the stored user, not from the old claims, so
    // a role change or a session revocation since sign-in is picked up here.
    let user = state
        .store
        .find_user_by_id(claims.user_id)
        .await?
        .ok_or(ApiError::Unauthorized)?;
    if user.token_version != claims.ver {
        return Err(ApiError::Unauthorized);
    }

    let response_headers = issue_session(&state, &user).await?;

    Ok((StatusCode::NO_CONTENT, response_headers))
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::response::Response;
    use axum::routing::post;
    use ironflow_auth::cookies::REFRESH_COOKIE_NAME;
    use ironflow_auth::jwt::{JwtConfig, RefreshToken};
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::context::WorkflowContext;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
    use ironflow_engine::notify::Event;
    use ironflow_store::entities::{NewUser, User};
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

    async fn signed_in_user(state: &AppState, is_admin: bool) -> (User, String) {
        let user = state
            .store
            .create_user(NewUser {
                email: "refresh@example.com".to_string(),
                username: "refreshuser".to_string(),
                password_hash: "argon2hash".to_string(),
                is_admin: Some(is_admin),
            })
            .await
            .expect("failed to create user");
        let headers = issue_session(state, &user)
            .await
            .expect("failed to issue session");
        let refresh_token = refresh_cookie(&headers);
        (user, refresh_token)
    }

    fn refresh_cookie(headers: &HeaderMap) -> String {
        let prefix = format!("{REFRESH_COOKIE_NAME}=");
        headers
            .get_all("set-cookie")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find_map(|c| c.strip_prefix(prefix.as_str()))
            .and_then(|rest| rest.split(';').next())
            .expect("refresh cookie not set")
            .to_string()
    }

    async fn post_refresh(state: &AppState, refresh_token: &str) -> Response {
        let app = Router::new()
            .route("/", post(refresh))
            .with_state(state.clone());

        let req = Request::builder()
            .uri("/")
            .method("POST")
            .header("content-type", "application/json")
            .header("Cookie", format!("{REFRESH_COOKIE_NAME}={refresh_token}"))
            .body(Body::empty())
            .expect("failed to build request");

        app.oneshot(req).await.expect("request failed")
    }

    #[tokio::test]
    async fn refresh_success() {
        let state = test_state();
        let (_, refresh_token) = signed_in_user(&state, false).await;

        let resp = post_refresh(&state, &refresh_token).await;
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        let set_cookie = resp.headers().get_all("set-cookie");
        assert_eq!(set_cookie.iter().count(), 2);
        assert_ne!(refresh_cookie(resp.headers()), refresh_token);
    }

    #[tokio::test]
    async fn rotated_refresh_token_is_accepted() {
        let state = test_state();
        let (_, refresh_token) = signed_in_user(&state, false).await;

        let first = post_refresh(&state, &refresh_token).await;
        assert_eq!(first.status(), StatusCode::NO_CONTENT);
        let rotated = refresh_cookie(first.headers());

        let second = post_refresh(&state, &rotated).await;
        assert_eq!(second.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn refresh_token_replayed_twice_is_rejected() {
        let state = test_state();
        let (_, refresh_token) = signed_in_user(&state, false).await;

        let first = post_refresh(&state, &refresh_token).await;
        assert_eq!(first.status(), StatusCode::NO_CONTENT);

        let replay = post_refresh(&state, &refresh_token).await;
        assert_eq!(replay.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn refresh_after_sign_out_is_rejected() {
        let state = test_state();
        let (user, refresh_token) = signed_in_user(&state, false).await;

        state
            .store
            .revoke_user_sessions(user.id)
            .await
            .expect("failed to revoke sessions");

        let resp = post_refresh(&state, &refresh_token).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn refresh_after_role_change_is_rejected() {
        let state = test_state();
        let (user, refresh_token) = signed_in_user(&state, true).await;

        state
            .store
            .update_user_role(user.id, false)
            .await
            .expect("failed to demote user");

        let resp = post_refresh(&state, &refresh_token).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn refresh_token_never_stored_is_rejected() {
        let state = test_state();
        let jwt_config = test_jwt_config();
        let user_id = Uuid::now_v7();

        // Correctly signed, but never recorded by a sign-in: the signature
        // alone no longer buys a session.
        let refresh_token = RefreshToken::for_user(user_id, "testuser", false, &jwt_config)
            .expect("failed to create refresh token");

        let resp = post_refresh(&state, &refresh_token.0).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
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
