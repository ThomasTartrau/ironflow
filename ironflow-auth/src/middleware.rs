//! Axum middleware for JWT authentication on protected routes.
//!
//! This middleware validates that a request contains a valid JWT token
//! (in a cookie or `Authorization: Bearer` header) before allowing the request
//! to reach the handler. It rejects with 401 if no token is found.
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
use serde_json::json;

use crate::cookies::AUTH_COOKIE_NAME;
use crate::jwt::{AccessToken, JwtConfig};

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
///
/// On success, the request proceeds to the handler.
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
/// The middleware will automatically extract `Arc<JwtConfig>` from the router state
/// via `FromRef`.
pub async fn jwt_auth<S>(State(state): State<S>, req: Request, next: Next) -> Response
where
    S: Send + Sync,
    Arc<JwtConfig>: FromRef<S>,
{
    let jwt_config = Arc::<JwtConfig>::from_ref(&state);

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

    match AccessToken::decode(&token, &jwt_config) {
        Ok(_) => next.run(req).await,
        Err(_) => (
            StatusCode::UNAUTHORIZED,
            Json(json!({
                "error": {
                    "code": "INVALID_TOKEN",
                    "message": "Invalid or expired authentication token",
                }
            })),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::body::Body;
    use axum::http::Request;
    use axum::middleware;
    use axum::routing::get;
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    use uuid::Uuid;

    fn test_jwt_config() -> Arc<JwtConfig> {
        Arc::new(JwtConfig {
            secret: "test-secret-key".to_string(),
            access_token_ttl_secs: 900,
            refresh_token_ttl_secs: 604800,
            cookie_domain: None,
            cookie_secure: false,
        })
    }

    async fn protected_handler() -> &'static str {
        "protected"
    }

    #[tokio::test]
    async fn rejects_missing_token() {
        let jwt_config = test_jwt_config();
        let app = Router::new()
            .route("/protected", get(protected_handler))
            .layer(middleware::from_fn_with_state(jwt_config.clone(), jwt_auth))
            .with_state(jwt_config);

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
        let jwt_config = test_jwt_config();
        let app = Router::new()
            .route("/protected", get(protected_handler))
            .layer(middleware::from_fn_with_state(jwt_config.clone(), jwt_auth))
            .with_state(jwt_config);

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
        let jwt_config = test_jwt_config();
        let user_id = Uuid::now_v7();
        let token = AccessToken::for_user(user_id, "testuser", false, &jwt_config).unwrap();

        let app = Router::new()
            .route("/protected", get(protected_handler))
            .layer(middleware::from_fn_with_state(jwt_config.clone(), jwt_auth))
            .with_state(jwt_config);

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
}
