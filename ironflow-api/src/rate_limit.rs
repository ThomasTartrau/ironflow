//! Per-IP rate limiting middleware backed by [`governor`].
//!
//! Use [`per_minute`] to create a limiter, then install the [`rate_limit`]
//! middleware on the desired router group. Rate-limited responses include
//! `X-RateLimit-Limit` / `Retry-After` headers and return HTTP 429 with a
//! JSON error body.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::num::NonZeroU32;
use std::sync::Arc;

use axum::extract::{ConnectInfo, Request};
use axum::http::{HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use governor::clock::{Clock, DefaultClock};
use governor::state::keyed::DashMapStateStore;
use governor::{Quota, RateLimiter};
use serde_json::json;

/// Keyed rate limiter: one bucket per IP address.
type KeyedLimiter = RateLimiter<IpAddr, DashMapStateStore<IpAddr>, DefaultClock>;

/// Shared rate limiter state, injected as an Axum extension.
#[derive(Clone)]
pub struct RateLimitState {
    limiter: Arc<KeyedLimiter>,
    burst: u32,
}

/// Build a per-IP rate limiter allowing `requests_per_minute` requests
/// per minute.
///
/// # Panics
///
/// Panics if `requests_per_minute` is 0.
///
/// # Examples
///
/// ```
/// use ironflow_api::rate_limit::per_minute;
///
/// let auth_limiter = per_minute(10);
/// let general_limiter = per_minute(60);
/// ```
pub fn per_minute(requests_per_minute: u32) -> RateLimitState {
    let quota = Quota::per_minute(NonZeroU32::new(requests_per_minute).expect("burst must be > 0"));
    RateLimitState {
        limiter: Arc::new(RateLimiter::keyed(quota)),
        burst: requests_per_minute,
    }
}

/// Axum middleware that enforces per-IP rate limiting.
///
/// Must be installed with a [`RateLimitState`] extension on the router.
/// Returns 429 Too Many Requests when the limit is exceeded, with a JSON body
/// and `X-RateLimit-Limit` / `Retry-After` headers.
pub async fn rate_limit(req: Request, next: Next) -> Response {
    let state = match req.extensions().get::<RateLimitState>() {
        Some(s) => s.clone(),
        None => return next.run(req).await,
    };

    let ip = extract_client_ip(&req);

    match state.limiter.check_key(&ip) {
        Ok(_) => {
            let mut resp = next.run(req).await;
            resp.headers_mut()
                .insert("x-ratelimit-limit", HeaderValue::from(state.burst));
            resp
        }
        Err(not_until) => {
            let retry_after = not_until.wait_time_from(DefaultClock::default().now());
            let retry_secs = retry_after.as_secs().max(1);

            let body = json!({
                "error": {
                    "code": "RATE_LIMIT_EXCEEDED",
                    "message": "Too many requests, please try again later",
                    "retry_after_secs": retry_secs,
                }
            });

            let mut resp = (StatusCode::TOO_MANY_REQUESTS, axum::Json(body)).into_response();
            resp.headers_mut()
                .insert("retry-after", HeaderValue::from(retry_secs));
            resp.headers_mut()
                .insert("x-ratelimit-limit", HeaderValue::from(state.burst));
            resp
        }
    }
}

/// Extract the client IP from the request.
///
/// Checks `X-Forwarded-For` first (first IP in the chain), then
/// `X-Real-Ip`, then falls back to the connected peer address.
fn extract_client_ip(req: &Request) -> IpAddr {
    // X-Forwarded-For: client, proxy1, proxy2
    if let Some(forwarded) = req
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        && let Some(first) = forwarded.split(',').next()
        && let Ok(ip) = first.trim().parse::<IpAddr>()
    {
        return ip;
    }

    // X-Real-Ip
    if let Some(real_ip) = req.headers().get("x-real-ip").and_then(|v| v.to_str().ok())
        && let Ok(ip) = real_ip.trim().parse::<IpAddr>()
    {
        return ip;
    }

    // Fallback to connected peer (axum injects ConnectInfo)
    req.extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip())
        .unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED))
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::middleware as axum_mw;
    use axum::routing::get;
    use axum::{Extension, Router};
    use http_body_util::BodyExt;
    use serde_json::Value as JsonValue;
    use tower::ServiceExt;

    use super::*;

    async fn ok_handler() -> &'static str {
        "ok"
    }

    fn test_app(limiter: RateLimitState) -> Router {
        Router::new()
            .route("/test", get(ok_handler))
            .layer(axum_mw::from_fn(rate_limit))
            .layer(Extension(limiter))
    }

    fn test_request() -> Request<Body> {
        Request::builder()
            .uri("/test")
            .header("x-forwarded-for", "1.2.3.4")
            .body(Body::empty())
            .unwrap()
    }

    #[tokio::test]
    async fn allows_requests_within_limit() {
        let limiter = per_minute(5);
        let app = test_app(limiter);

        let resp = app.oneshot(test_request()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(resp.headers().contains_key("x-ratelimit-limit"));
    }

    #[tokio::test]
    async fn rejects_when_limit_exceeded() {
        let limiter = per_minute(2);
        let app = test_app(limiter.clone());

        // Exhaust the limiter from the same IP
        let _ = app.oneshot(test_request()).await;
        let app = test_app(limiter.clone());
        let _ = app.oneshot(test_request()).await;

        let app = test_app(limiter);
        let resp = app.oneshot(test_request()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = serde_json::from_slice(&body).unwrap();
        assert_eq!(json_val["error"]["code"], "RATE_LIMIT_EXCEEDED");
    }

    #[tokio::test]
    async fn includes_retry_after_header() {
        let limiter = per_minute(1);
        let app = test_app(limiter.clone());

        // Use up the single allowed request
        let _ = app.oneshot(test_request()).await;

        let app = test_app(limiter);
        let resp = app.oneshot(test_request()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
        assert!(resp.headers().contains_key("retry-after"));
    }

    #[tokio::test]
    async fn different_ips_have_separate_limits() {
        let limiter = per_minute(1);

        let app = test_app(limiter.clone());
        let req_ip1 = Request::builder()
            .uri("/test")
            .header("x-forwarded-for", "10.0.0.1")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req_ip1).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let app = test_app(limiter);
        let req_ip2 = Request::builder()
            .uri("/test")
            .header("x-forwarded-for", "10.0.0.2")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req_ip2).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn extracts_ip_from_x_real_ip() {
        let limiter = per_minute(1);

        let app = test_app(limiter.clone());
        let req = Request::builder()
            .uri("/test")
            .header("x-real-ip", "192.168.1.1")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let app = test_app(limiter);
        let req = Request::builder()
            .uri("/test")
            .header("x-real-ip", "192.168.1.1")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn general_limiter_allows_more_requests() {
        let limiter = per_minute(60);

        for _ in 0..10 {
            let app = test_app(limiter.clone());
            let resp = app.oneshot(test_request()).await.unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
        }
    }

    #[test]
    fn extract_ip_x_forwarded_for_first_ip() {
        let req = Request::builder()
            .uri("/test")
            .header("x-forwarded-for", "1.2.3.4, 5.6.7.8")
            .body(Body::empty())
            .unwrap();
        let ip = extract_client_ip(&req);
        assert_eq!(ip, "1.2.3.4".parse::<IpAddr>().unwrap());
    }

    #[test]
    fn extract_ip_fallback_to_unspecified() {
        let req = Request::builder().uri("/test").body(Body::empty()).unwrap();
        let ip = extract_client_ip(&req);
        assert_eq!(ip, IpAddr::V4(Ipv4Addr::UNSPECIFIED));
    }
}
