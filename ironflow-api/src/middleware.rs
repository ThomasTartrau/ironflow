//! Middleware for internal route protection and HTTP security hardening.

use axum::Json;
use axum::extract::Request;
use axum::http::header::{
    CONTENT_SECURITY_POLICY, STRICT_TRANSPORT_SECURITY, X_CONTENT_TYPE_OPTIONS, X_FRAME_OPTIONS,
    X_XSS_PROTECTION,
};
use axum::http::{HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use subtle::ConstantTimeEq;

/// Axum middleware that validates a static worker token.
///
/// Extracts `Authorization: Bearer {token}` and compares against the
/// expected token. Returns 401 if missing or invalid.
pub async fn worker_token_auth(req: Request, next: Next) -> Response {
    let expected = req.extensions().get::<WorkerToken>().map(|t| t.0.clone());

    let provided = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|t| t.to_string());

    match (expected, provided) {
        (Some(expected), Some(provided))
            if expected.as_bytes().ct_eq(provided.as_bytes()).into() =>
        {
            next.run(req).await
        }
        _ => (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({
                "error": {
                    "code": "INVALID_WORKER_TOKEN",
                    "message": "Invalid or missing worker token",
                }
            })),
        )
            .into_response(),
    }
}

/// Newtype wrapper for the static worker token, stored in request extensions.
#[derive(Clone)]
pub struct WorkerToken(pub String);

/// Middleware that injects standard HTTP security headers on every response.
///
/// Headers set:
/// - `X-Content-Type-Options: nosniff` — prevents MIME-type sniffing
/// - `X-Frame-Options: DENY` — blocks clickjacking via iframes
/// - `X-XSS-Protection: 1; mode=block` — legacy XSS filter hint
/// - `Strict-Transport-Security: max-age=63072000; includeSubDomains` — enforces HTTPS for 2 years
/// - `Content-Security-Policy: default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self' data:; connect-src 'self'`
pub async fn security_headers(req: Request, next: Next) -> Response {
    let mut resp = next.run(req).await;
    let headers = resp.headers_mut();

    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(X_XSS_PROTECTION, HeaderValue::from_static("1; mode=block"));
    headers.insert(
        STRICT_TRANSPORT_SECURITY,
        HeaderValue::from_static("max-age=63072000; includeSubDomains"),
    );
    headers.insert(
        CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self' data:; connect-src 'self'",
        ),
    );

    resp
}

#[cfg(test)]
mod tests {

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::engine::Engine;
    use ironflow_store::memory::InMemoryStore;
    use std::sync::Arc;
    use tower::ServiceExt;

    use crate::routes::create_router;
    use crate::state::AppState;

    fn test_state() -> AppState {
        let store = Arc::new(InMemoryStore::new());
        let user_store = Arc::new(InMemoryStore::new());
        let provider = Arc::new(ClaudeCodeProvider::new());
        let engine = Arc::new(Engine::new(store.clone(), provider));
        let jwt_config = Arc::new(ironflow_auth::jwt::JwtConfig {
            secret: "test-secret".to_string(),
            access_token_ttl_secs: 900,
            refresh_token_ttl_secs: 604800,
            cookie_domain: None,
            cookie_secure: false,
        });
        AppState {
            store,
            user_store,
            engine,
            jwt_config,
            worker_token: "test-worker-token".to_string(),
        }
    }

    #[tokio::test]
    async fn worker_token_valid() {
        let state = test_state();
        let app = create_router(state.clone(), None);

        let req = Request::builder()
            .uri("/api/v1/internal/runs/next")
            .header("authorization", "Bearer test-worker-token")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn worker_token_missing() {
        let state = test_state();
        let app = create_router(state, None);

        let req = Request::builder()
            .uri("/api/v1/internal/runs/next")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"]["code"], "INVALID_WORKER_TOKEN");
    }

    #[tokio::test]
    async fn worker_token_invalid() {
        let state = test_state();
        let app = create_router(state, None);

        let req = Request::builder()
            .uri("/api/v1/internal/runs/next")
            .header("authorization", "Bearer wrong-token")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"]["code"], "INVALID_WORKER_TOKEN");
    }

    #[tokio::test]
    async fn security_headers_present() {
        let state = test_state();
        let app = create_router(state, None);

        let req = Request::builder()
            .uri("/api/v1/health-check")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(
            resp.headers().get("x-content-type-options").unwrap(),
            "nosniff"
        );
        assert_eq!(resp.headers().get("x-frame-options").unwrap(), "DENY");
        assert_eq!(
            resp.headers().get("x-xss-protection").unwrap(),
            "1; mode=block"
        );
        assert_eq!(
            resp.headers().get("strict-transport-security").unwrap(),
            "max-age=63072000; includeSubDomains"
        );
        assert!(
            resp.headers()
                .get("content-security-policy")
                .unwrap()
                .to_str()
                .unwrap()
                .contains("default-src 'self'")
        );
    }
}
