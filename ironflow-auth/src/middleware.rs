//! Axum middleware for JWT authentication on protected routes.
//!
//! This middleware validates that a request contains a valid JWT token
//! (in a cookie or `Authorization: Bearer` header) before allowing the request
//! to reach the handler. It rejects with 401 if no token is found, if the
//! token is invalid, or if the user's sessions have been revoked.
//!
//! Use this to protect route groups without adding `AuthenticatedUser` as a parameter
//! to every handler.

use std::sync::Arc;

use axum::Json;
use axum::extract::Request;
use axum::extract::{FromRef, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum_extra::extract::CookieJar;
use ironflow_store::store::Store;
use serde_json::json;

use crate::cookies::AUTH_COOKIE_NAME;
use crate::extractor::verify_access_token;
use crate::jwt::JwtConfig;

/// Extract and validate a JWT token from request headers.
///
/// Checks the `authorization` header for `Bearer <token>` and falls back to
/// the auth cookie. Returns the token string on success.
fn extract_token(headers: &HeaderMap) -> Option<String> {
    let jar = CookieJar::from_headers(headers);
    jar.get(AUTH_COOKIE_NAME)
        .map(|c| c.value().to_string())
        .or_else(|| {
            headers
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("Bearer "))
                .map(|t| t.to_string())
        })
}

/// Axum middleware that enforces JWT authentication.
///
/// Validates that a request contains a valid JWT token and rejects with 401 if:
/// - No token is present in cookies or `Authorization` header
/// - The token is invalid or expired
/// - The token was issued before the user's sessions were revoked
///
/// On success, the request proceeds to the handler. A store failure while
/// looking up the user rejects with 500.
///
/// # Examples
///
/// ```no_run
/// use axum::Router;
/// use axum::routing::get;
/// use axum::middleware;
/// use ironflow_auth::middleware::jwt_auth;
/// use ironflow_auth::jwt::JwtConfig;
/// use std::sync::Arc;
///
/// # async fn example(jwt_config: Arc<JwtConfig>) {
/// // In your route setup, layer the middleware with the state:
/// // .layer(middleware::from_fn_with_state(state, jwt_auth))
/// # }
/// ```
///
/// The middleware will automatically extract `Arc<JwtConfig>` and
/// `Arc<dyn Store>` from the router state via `FromRef`.
pub async fn jwt_auth<S>(State(state): State<S>, req: Request, next: Next) -> Response
where
    S: Send + Sync,
    Arc<JwtConfig>: FromRef<S>,
    Arc<dyn Store>: FromRef<S>,
{
    let jwt_config = Arc::<JwtConfig>::from_ref(&state);
    let store = Arc::<dyn Store>::from_ref(&state);

    let token = match extract_token(req.headers()) {
        Some(t) => t,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({
                    "error": {
                        "code": "MISSING_TOKEN",
                        "message": "No authentication token provided",
                    }
                })),
            )
                .into_response();
        }
    };

    match verify_access_token(&token, &jwt_config, store.as_ref()).await {
        Ok(_) => next.run(req).await,
        Err(rejection) => rejection.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use axum::Router;
    use axum::body::Body;
    use axum::http::Request;
    use axum::middleware;
    use axum::routing::get;
    use http_body_util::BodyExt;
    use ironflow_store::entities::NewUser;
    use ironflow_store::memory::InMemoryStore;
    use tower::ServiceExt;
    use uuid::Uuid;

    use crate::jwt::AccessToken;

    #[derive(Clone)]
    struct TestState {
        jwt_config: Arc<JwtConfig>,
        store: Arc<dyn Store>,
    }

    impl FromRef<TestState> for Arc<JwtConfig> {
        fn from_ref(state: &TestState) -> Self {
            state.jwt_config.clone()
        }
    }

    impl FromRef<TestState> for Arc<dyn Store> {
        fn from_ref(state: &TestState) -> Self {
            state.store.clone()
        }
    }

    fn test_jwt_config() -> Arc<JwtConfig> {
        Arc::new(JwtConfig {
            secret: "test-secret-key".to_string(),
            access_token_ttl_secs: 900,
            refresh_token_ttl_secs: 604800,
            cookie_domain: None,
            cookie_secure: false,
        })
    }

    fn test_state() -> TestState {
        TestState {
            jwt_config: test_jwt_config(),
            store: Arc::new(InMemoryStore::new()),
        }
    }

    async fn protected_handler() -> &'static str {
        "protected"
    }

    fn protected_app(state: TestState) -> Router {
        Router::new()
            .route("/protected", get(protected_handler))
            .layer(middleware::from_fn_with_state(
                state.clone(),
                jwt_auth::<TestState>,
            ))
            .with_state(state)
    }

    #[tokio::test]
    async fn rejects_missing_token() {
        let app = protected_app(test_state());

        let req = Request::builder()
            .uri("/protected")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"]["code"], "MISSING_TOKEN");
    }

    #[tokio::test]
    async fn rejects_invalid_token() {
        let app = protected_app(test_state());

        let req = Request::builder()
            .uri("/protected")
            .header("authorization", "Bearer not.a.valid.token")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"]["code"], "INVALID_TOKEN");
    }

    #[tokio::test]
    async fn allows_valid_bearer_token() {
        let state = test_state();
        let user_id = Uuid::now_v7();
        let token = AccessToken::for_user(user_id, "testuser", false, &state.jwt_config).unwrap();

        let app = protected_app(state);

        let req = Request::builder()
            .uri("/protected")
            .header("authorization", format!("Bearer {}", token.0))
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], b"protected");
    }

    #[tokio::test]
    async fn rejects_revoked_token() {
        let state = test_state();
        let user = state
            .store
            .create_user(NewUser {
                email: "revoked@test.com".to_string(),
                username: "revoked".to_string(),
                password_hash: "argon2hash".to_string(),
                is_admin: Some(false),
            })
            .await
            .unwrap();
        let token = AccessToken::for_user(user.id, "revoked", false, &state.jwt_config).unwrap();
        state.store.revoke_user_sessions(user.id).await.unwrap();

        let app = protected_app(state);

        let req = Request::builder()
            .uri("/protected")
            .header("authorization", format!("Bearer {}", token.0))
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"]["code"], "TOKEN_REVOKED");
    }
}
