//! Middleware for internal route protection and HTTP security hardening.

use axum::Json;
use axum::extract::Request;
use axum::http::header::{
    CONTENT_SECURITY_POLICY, HOST, LOCATION, STRICT_TRANSPORT_SECURITY, X_CONTENT_TYPE_OPTIONS,
    X_FRAME_OPTIONS, X_XSS_PROTECTION,
};
use axum::http::{HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde_json::json;
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
            Json(json!({
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

/// Middleware that records API request metrics (counter + duration histogram).
///
/// Emits `ironflow_api_requests_total` and `ironflow_api_request_duration_seconds`
/// for every request. Only compiled when the `prometheus` feature is enabled.
#[cfg(feature = "prometheus")]
pub async fn request_metrics(req: Request, next: Next) -> Response {
    use std::time::Instant;

    use ironflow_core::metric_names::{API_REQUEST_DURATION_SECONDS, API_REQUESTS_TOTAL};
    use metrics::{counter, histogram};

    let method = req.method().to_string();
    let path = req.uri().path().to_string();
    let start = Instant::now();

    let resp = next.run(req).await;

    let status = resp.status().as_u16().to_string();
    let duration = start.elapsed().as_secs_f64();

    counter!(API_REQUESTS_TOTAL, "method" => method.clone(), "path" => path.clone(), "status" => status).increment(1);
    histogram!(API_REQUEST_DURATION_SECONDS, "method" => method, "path" => path).record(duration);

    resp
}

/// Middleware that redirects plain-HTTP requests to HTTPS.
///
/// TLS terminates at a reverse proxy, so plain HTTP is detected through the
/// `X-Forwarded-Proto` header (first value of a comma-separated list,
/// case-insensitive). When it is `http`, the response is a
/// `308 Permanent Redirect` to `https://{host}{path_and_query}`, which keeps
/// the method and body. The host comes from `X-Forwarded-Host`, falling back
/// to `Host`. Requests without `X-Forwarded-Proto` (probes, worker traffic) and
/// requests with no usable host pass through untouched.
///
/// # Examples
///
/// ```
/// use axum::Router;
/// use axum::middleware::from_fn;
/// use axum::routing::get;
/// use ironflow_api::middleware::https_redirect;
///
/// let app: Router = Router::new()
///     .route("/", get(|| async { "ok" }))
///     .layer(from_fn(https_redirect));
/// ```
pub async fn https_redirect(req: Request, next: Next) -> Response {
    let is_http = req
        .headers()
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("http"));

    if is_http {
        let host = req
            .headers()
            .get("x-forwarded-host")
            .or_else(|| req.headers().get(HOST))
            .and_then(|v| v.to_str().ok())
            .map(|v| v.split(',').next().unwrap_or(v).trim())
            .filter(|v| !v.is_empty());
        let path = req.uri().path_and_query().map_or("/", |pq| pq.as_str());

        if let Some(host) = host
            && let Ok(location) = HeaderValue::from_str(&format!("https://{host}{path}"))
        {
            let mut resp = StatusCode::PERMANENT_REDIRECT.into_response();
            resp.headers_mut().insert(LOCATION, location);
            return resp;
        }
    }

    next.run(req).await
}

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

    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::notify::Event;
    use ironflow_store::memory::InMemoryStore;
    use serde_json::Value as JsonValue;
    use std::fs::write;
    use std::sync::Arc;
    use tempfile::TempDir;
    use tokio::sync::broadcast;
    use tower::ServiceExt;

    use crate::routes::{RouterConfig, create_router};
    use crate::state::AppState;

    fn test_state() -> AppState {
        let store = Arc::new(InMemoryStore::new());
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
    async fn worker_token_valid() {
        let state = test_state();
        let app = create_router(state.clone(), RouterConfig::default());

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
        let app = create_router(state, RouterConfig::default());

        let req = Request::builder()
            .uri("/api/v1/internal/runs/next")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = serde_json::from_slice(&body).unwrap();
        assert_eq!(json_val["error"]["code"], "INVALID_WORKER_TOKEN");
    }

    #[tokio::test]
    async fn worker_token_invalid() {
        let state = test_state();
        let app = create_router(state, RouterConfig::default());

        let req = Request::builder()
            .uri("/api/v1/internal/runs/next")
            .header("authorization", "Bearer wrong-token")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = serde_json::from_slice(&body).unwrap();
        assert_eq!(json_val["error"]["code"], "INVALID_WORKER_TOKEN");
    }

    fn https_request(method: &str, proto: Option<&str>) -> Request<Body> {
        let mut builder = Request::builder()
            .method(method)
            .uri("/api/v1/health-check?x=1")
            .header("host", "example.com");
        if let Some(proto) = proto {
            builder = builder.header("x-forwarded-proto", proto);
        }
        builder.body(Body::empty()).unwrap()
    }

    fn enforcing_router() -> Router {
        let config = RouterConfig {
            enforce_https: true,
            ..RouterConfig::default()
        };
        create_router(test_state(), config)
    }

    #[tokio::test]
    async fn https_redirect_redirects_plain_http() {
        let resp = enforcing_router()
            .oneshot(https_request("GET", Some("http")))
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::PERMANENT_REDIRECT);
        assert_eq!(
            resp.headers().get("location").unwrap(),
            "https://example.com/api/v1/health-check?x=1"
        );
        assert!(resp.headers().get("strict-transport-security").is_some());
    }

    #[tokio::test]
    async fn https_redirect_uses_first_value_and_ignores_case() {
        let resp = enforcing_router()
            .oneshot(https_request("GET", Some("HTTP, https")))
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::PERMANENT_REDIRECT);
    }

    #[tokio::test]
    async fn https_redirect_prefers_forwarded_host() {
        let req = Request::builder()
            .uri("/api/v1/health-check")
            .header("host", "internal:3000")
            .header("x-forwarded-host", "public.example.com")
            .header("x-forwarded-proto", "http")
            .body(Body::empty())
            .unwrap();

        let resp = enforcing_router().oneshot(req).await.unwrap();

        assert_eq!(resp.status(), StatusCode::PERMANENT_REDIRECT);
        assert_eq!(
            resp.headers().get("location").unwrap(),
            "https://public.example.com/api/v1/health-check"
        );
    }

    #[tokio::test]
    async fn https_redirect_passes_https_through() {
        let resp = enforcing_router()
            .oneshot(https_request("GET", Some("https")))
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn https_redirect_passes_without_forwarded_proto() {
        let resp = enforcing_router()
            .oneshot(https_request("GET", None))
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn https_redirect_without_host_passes_through() {
        let req = Request::builder()
            .uri("/api/v1/health-check")
            .header("x-forwarded-proto", "http")
            .body(Body::empty())
            .unwrap();

        let resp = enforcing_router().oneshot(req).await.unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn https_redirect_disabled_by_default() {
        let app = create_router(test_state(), RouterConfig::default());

        let resp = app
            .oneshot(https_request("GET", Some("http")))
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn https_redirect_post_gets_308() {
        let resp = enforcing_router()
            .oneshot(https_request("POST", Some("http")))
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::PERMANENT_REDIRECT);
    }

    /// Router serving a dashboard from a temporary directory. The directory
    /// is returned so it outlives the router.
    fn dashboard_router(enforce_https: bool) -> (Router, TempDir) {
        let dir = TempDir::new().unwrap();
        write(dir.path().join("index.html"), "<!doctype html>").unwrap();
        let config = RouterConfig {
            enforce_https,
            dashboard_dir: Some(dir.path().to_path_buf()),
            ..RouterConfig::default()
        };
        (create_router(test_state(), config), dir)
    }

    #[tokio::test]
    async fn https_redirect_covers_dashboard() {
        // Non-regression: the dashboard fallback used to be attached after the
        // layers, so plain HTTP on the dashboard was served without redirect.
        let (app, _dir) = dashboard_router(true);
        let req = Request::builder()
            .uri("/runs?page=2")
            .header("host", "example.com")
            .header("x-forwarded-proto", "http")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), StatusCode::PERMANENT_REDIRECT);
        assert_eq!(
            resp.headers().get("location").unwrap(),
            "https://example.com/runs?page=2"
        );
    }

    #[tokio::test]
    async fn security_headers_cover_dashboard() {
        let (app, _dir) = dashboard_router(false);
        let req = Request::builder().uri("/").body(Body::empty()).unwrap();

        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers().get("x-frame-options").unwrap(), "DENY");
        assert!(resp.headers().get("strict-transport-security").is_some());
        assert!(resp.headers().get("content-security-policy").is_some());
    }

    #[tokio::test]
    async fn security_headers_present() {
        let state = test_state();
        let app = create_router(state, RouterConfig::default());

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
