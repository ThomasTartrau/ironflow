//! Identity-aware rate limiting middleware.
//!
//! Requests are keyed by authenticated identity (API key ID or user ID) when
//! available, falling back to the client IP address. Each key gets a
//! fixed-window counter that resets every 60 seconds.
//!
//! Responses carry standard rate-limit headers:
//! - `X-RateLimit-Limit` -- maximum requests allowed per window
//! - `X-RateLimit-Remaining` -- requests left in the current window
//! - `X-RateLimit-Reset` -- epoch timestamp when the window resets
//!
//! API keys with a `rate_limit_override` use that value instead of the
//! server-wide default.

use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::Request;
use axum::http::{HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use dashmap::DashMap;
use serde_json::json;
use uuid::Uuid;

use ironflow_auth::extractor::{API_KEY_PREFIX, API_KEY_SUFFIX_LEN};
use ironflow_auth::jwt::{AccessToken, JwtConfig};
use ironflow_store::store::Store;

/// Rate limit key: identity-based when authenticated, IP-based otherwise.
///
/// # Examples
///
/// ```
/// use std::net::{IpAddr, Ipv4Addr};
/// use uuid::Uuid;
/// use ironflow_api::rate_limit::RateLimitKey;
///
/// let ip_key = RateLimitKey::Ip(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)));
/// let user_key = RateLimitKey::User(Uuid::nil());
/// let api_key = RateLimitKey::ApiKey(Uuid::nil());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RateLimitKey {
    /// Authenticated via API key.
    ApiKey(Uuid),
    /// Authenticated via JWT.
    User(Uuid),
    /// Unauthenticated, identified by client IP.
    Ip(IpAddr),
}

struct WindowEntry {
    count: AtomicU32,
    window_start: AtomicU64,
}

/// Shared rate limiter state with fixed-window counters.
///
/// # Examples
///
/// ```
/// use ironflow_api::rate_limit::per_minute;
///
/// let limiter = per_minute(60);
/// ```
#[derive(Clone)]
pub struct RateLimitState {
    counters: Arc<DashMap<RateLimitKey, WindowEntry>>,
    burst: u32,
}

/// Context needed by the rate limit middleware.
///
/// Combines the limiter state with auth dependencies for identity extraction.
///
/// # Examples
///
/// ```no_run
/// use std::sync::Arc;
/// use ironflow_api::rate_limit::{RateLimitContext, per_minute};
/// use ironflow_auth::jwt::JwtConfig;
/// use ironflow_store::memory::InMemoryStore;
/// use ironflow_store::store::Store;
///
/// let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
/// let jwt_config = Arc::new(JwtConfig {
///     secret: "secret".to_string(),
///     access_token_ttl_secs: 900,
///     refresh_token_ttl_secs: 604800,
///     cookie_domain: None,
///     cookie_secure: false,
/// });
/// let ctx = RateLimitContext {
///     store,
///     jwt_config,
///     limiter: per_minute(60),
/// };
/// ```
#[derive(Clone)]
pub struct RateLimitContext {
    /// Backing store for API key prefix lookups.
    pub store: Arc<dyn Store>,
    /// JWT config for token decoding.
    pub jwt_config: Arc<JwtConfig>,
    /// The rate limiter itself.
    pub limiter: RateLimitState,
}

struct RateLimitResult {
    limit: u32,
    remaining: u32,
    reset: u64,
    allowed: bool,
}

/// Build a per-identity rate limiter allowing `requests_per_minute` requests
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
    assert!(requests_per_minute > 0, "burst must be > 0");
    RateLimitState {
        counters: Arc::new(DashMap::new()),
        burst: requests_per_minute,
    }
}

const WINDOW_SECS: u64 = 60;

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before UNIX epoch")
        .as_secs()
}

fn check_rate_limit(limiter: &RateLimitState, key: RateLimitKey, limit: u32) -> RateLimitResult {
    let now = now_epoch();

    let entry = limiter.counters.entry(key).or_insert_with(|| WindowEntry {
        count: AtomicU32::new(0),
        window_start: AtomicU64::new(now),
    });

    let window_start = entry.window_start.load(Ordering::Acquire);
    if now >= window_start + WINDOW_SECS
        && entry
            .window_start
            .compare_exchange(window_start, now, Ordering::AcqRel, Ordering::Relaxed)
            .is_ok()
    {
        entry.count.store(0, Ordering::Release);
    }

    let count = entry.count.fetch_add(1, Ordering::AcqRel) + 1;
    let reset = entry.window_start.load(Ordering::Acquire) + WINDOW_SECS;

    if count > limit {
        entry.count.fetch_sub(1, Ordering::AcqRel);
        RateLimitResult {
            limit,
            remaining: 0,
            reset,
            allowed: false,
        }
    } else {
        RateLimitResult {
            limit,
            remaining: limit.saturating_sub(count),
            reset,
            allowed: true,
        }
    }
}

/// Axum middleware that enforces identity-aware rate limiting.
///
/// Extracts the caller's identity from the request:
/// - API key (`irfl_*` Bearer token) -> `RateLimitKey::ApiKey(id)`
/// - JWT (Bearer token or cookie) -> `RateLimitKey::User(id)`
/// - Fallback -> `RateLimitKey::Ip(addr)`
///
/// API keys with a `rate_limit_override` use that value instead of
/// the default burst.
pub async fn rate_limit(mut req: Request, next: Next) -> Response {
    let ctx = req.extensions_mut().remove::<RateLimitContext>();
    let Some(ctx) = ctx else {
        return next.run(req).await;
    };
    let (key, override_limit) = extract_rate_limit_key_from_headers(req.headers(), &ctx).await;
    let limit = override_limit.unwrap_or(ctx.limiter.burst);

    if limit == 0 {
        return next.run(req).await;
    }

    let result = check_rate_limit(&ctx.limiter, key, limit);

    if result.allowed {
        let mut resp = next.run(req).await;
        insert_rate_limit_headers(resp.headers_mut(), &result);
        resp
    } else {
        let retry_after = result.reset.saturating_sub(now_epoch()).max(1);

        let body = json!({
            "error": {
                "code": "RATE_LIMIT_EXCEEDED",
                "message": "Too many requests, please try again later",
                "retry_after_secs": retry_after,
            }
        });

        let mut resp = (StatusCode::TOO_MANY_REQUESTS, axum::Json(body)).into_response();
        resp.headers_mut()
            .insert("retry-after", HeaderValue::from(retry_after));
        insert_rate_limit_headers(resp.headers_mut(), &result);
        resp
    }
}

fn insert_rate_limit_headers(headers: &mut axum::http::HeaderMap, result: &RateLimitResult) {
    headers.insert("x-ratelimit-limit", HeaderValue::from(result.limit));
    headers.insert("x-ratelimit-remaining", HeaderValue::from(result.remaining));
    headers.insert("x-ratelimit-reset", HeaderValue::from(result.reset));
}

/// Extract the rate limit key and optional override from the request.
///
/// Attempts lightweight identity extraction without full auth verification:
/// - API key prefix lookup (cheap index scan, no argon2)
/// - JWT decode (local crypto, no DB)
/// - Fallback to client IP
async fn extract_rate_limit_key_from_headers(
    headers: &axum::http::HeaderMap,
    ctx: &RateLimitContext,
) -> (RateLimitKey, Option<u32>) {
    let bearer = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|s| s.to_string());

    if let Some(ref token) = bearer {
        if token.starts_with(API_KEY_PREFIX) {
            if let Some((key, override_limit)) = try_api_key_identity(token, ctx).await {
                return (key, override_limit);
            }
        } else if let Some(key) = try_jwt_identity(token, ctx) {
            return (key, None);
        }
    }

    // Try JWT from cookie
    if let Some(cookie_header) = headers.get("cookie").and_then(|v| v.to_str().ok()) {
        for part in cookie_header.split(';') {
            let part = part.trim();
            if let Some(value) = part.strip_prefix("ironflow_session=")
                && let Some(key) = try_jwt_identity(value, ctx)
            {
                return (key, None);
            }
        }
    }

    let ip = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .and_then(|s| s.trim().parse::<IpAddr>().ok())
        .or_else(|| {
            headers
                .get("x-real-ip")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.trim().parse::<IpAddr>().ok())
        })
        .unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED));

    (RateLimitKey::Ip(ip), None)
}

async fn try_api_key_identity(
    token: &str,
    ctx: &RateLimitContext,
) -> Option<(RateLimitKey, Option<u32>)> {
    let suffix_len = (token.len() - API_KEY_PREFIX.len()).min(API_KEY_SUFFIX_LEN);
    let prefix = &token[..API_KEY_PREFIX.len() + suffix_len];

    let api_key = ctx.store.find_api_key_by_prefix(prefix).await.ok()??;

    let override_limit = api_key.rate_limit_override;
    Some((RateLimitKey::ApiKey(api_key.id), override_limit))
}

fn try_jwt_identity(token: &str, ctx: &RateLimitContext) -> Option<RateLimitKey> {
    let claims = AccessToken::decode(token, &ctx.jwt_config).ok()?;
    Some(RateLimitKey::User(claims.user_id))
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use axum::extract::ConnectInfo;

    fn extract_client_ip(req: &Request<Body>) -> IpAddr {
        if let Some(forwarded) = req
            .headers()
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            && let Some(first) = forwarded.split(',').next()
            && let Ok(ip) = first.trim().parse::<IpAddr>()
        {
            return ip;
        }

        if let Some(real_ip) = req.headers().get("x-real-ip").and_then(|v| v.to_str().ok())
            && let Ok(ip) = real_ip.trim().parse::<IpAddr>()
        {
            return ip;
        }

        req.extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map(|ci| ci.0.ip())
            .unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED))
    }

    use axum::Extension;
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::middleware as axum_mw;
    use axum::routing::get;
    use http_body_util::BodyExt;
    use ironflow_auth::password;
    use ironflow_store::entities::{ApiKeyScope, NewApiKey, NewUser};
    use ironflow_store::memory::InMemoryStore;
    use serde_json::Value as JsonValue;
    use tower::ServiceExt;

    use super::*;

    async fn ok_handler() -> &'static str {
        "ok"
    }

    fn test_jwt_config() -> Arc<JwtConfig> {
        Arc::new(JwtConfig {
            secret: "test-secret-for-rate-limit".to_string(),
            access_token_ttl_secs: 900,
            refresh_token_ttl_secs: 604800,
            cookie_domain: None,
            cookie_secure: false,
        })
    }

    fn test_ctx(burst: u32) -> RateLimitContext {
        let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
        RateLimitContext {
            store,
            jwt_config: test_jwt_config(),
            limiter: per_minute(burst),
        }
    }

    fn test_app(ctx: RateLimitContext) -> Router {
        Router::new()
            .route("/test", get(ok_handler))
            .layer(axum_mw::from_fn(rate_limit))
            .layer(Extension(ctx))
    }

    fn ip_request(ip: &str) -> Request<Body> {
        Request::builder()
            .uri("/test")
            .header("x-forwarded-for", ip)
            .body(Body::empty())
            .unwrap()
    }

    async fn setup_api_key_in_store(
        store: &Arc<dyn Store>,
        rate_limit_override: Option<u32>,
    ) -> (Uuid, String) {
        let user = store
            .create_user(NewUser {
                email: "rl-test@test.com".to_string(),
                username: "rl-test".to_string(),
                password_hash: password::hash("pass").unwrap(),
                is_admin: Some(false),
            })
            .await
            .unwrap();

        let raw_key = "irfl_abcdef12rest-of-secret-key";
        let key_hash = password::hash(raw_key).unwrap();
        let prefix =
            &raw_key[..API_KEY_PREFIX.len() + ironflow_auth::extractor::API_KEY_SUFFIX_LEN];

        let api_key = store
            .create_api_key(NewApiKey {
                user_id: user.id,
                name: "rl-test-key".to_string(),
                key_hash,
                key_prefix: prefix.to_string(),
                scopes: vec![ApiKeyScope::RunsRead],
                expires_at: None,
                rate_limit_override,
            })
            .await
            .unwrap();

        (api_key.id, raw_key.to_string())
    }

    #[tokio::test]
    async fn unauthenticated_uses_ip_bucket() {
        let ctx = test_ctx(2);
        let app = test_app(ctx.clone());

        let resp = app.oneshot(ip_request("1.2.3.4")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let app = test_app(ctx.clone());
        let resp = app.oneshot(ip_request("1.2.3.4")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let app = test_app(ctx);
        let resp = app.oneshot(ip_request("1.2.3.4")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn api_key_auth_uses_separate_bucket() {
        let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
        let (_api_key_id, raw_key) = setup_api_key_in_store(&store, None).await;

        let ctx = RateLimitContext {
            store,
            jwt_config: test_jwt_config(),
            limiter: per_minute(2),
        };

        // Use up both IP requests from 1.2.3.4
        let app = test_app(ctx.clone());
        let _ = app.oneshot(ip_request("1.2.3.4")).await;
        let app = test_app(ctx.clone());
        let _ = app.oneshot(ip_request("1.2.3.4")).await;

        // IP 1.2.3.4 is now exhausted
        let app = test_app(ctx.clone());
        let resp = app.oneshot(ip_request("1.2.3.4")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);

        // But API key from same IP still works (separate bucket)
        let app = test_app(ctx);
        let req = Request::builder()
            .uri("/test")
            .header("x-forwarded-for", "1.2.3.4")
            .header("authorization", format!("Bearer {raw_key}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn jwt_auth_uses_user_bucket() {
        let ctx = test_ctx(2);
        let user_id = Uuid::now_v7();
        let token = AccessToken::for_user(user_id, "alice", false, &ctx.jwt_config).unwrap();

        // Use up both IP requests
        let app = test_app(ctx.clone());
        let _ = app.oneshot(ip_request("10.0.0.1")).await;
        let app = test_app(ctx.clone());
        let _ = app.oneshot(ip_request("10.0.0.1")).await;

        // IP exhausted
        let app = test_app(ctx.clone());
        let resp = app.oneshot(ip_request("10.0.0.1")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);

        // JWT from same IP still works (separate User bucket)
        let app = test_app(ctx);
        let req = Request::builder()
            .uri("/test")
            .header("x-forwarded-for", "10.0.0.1")
            .header("authorization", format!("Bearer {}", token.0))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn remaining_header_decrements() {
        let ctx = test_ctx(3);

        let app = test_app(ctx.clone());
        let resp = app.oneshot(ip_request("2.2.2.2")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let remaining: u32 = resp
            .headers()
            .get("x-ratelimit-remaining")
            .unwrap()
            .to_str()
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(remaining, 2);

        let app = test_app(ctx.clone());
        let resp = app.oneshot(ip_request("2.2.2.2")).await.unwrap();
        let remaining: u32 = resp
            .headers()
            .get("x-ratelimit-remaining")
            .unwrap()
            .to_str()
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(remaining, 1);

        let app = test_app(ctx);
        let resp = app.oneshot(ip_request("2.2.2.2")).await.unwrap();
        let remaining: u32 = resp
            .headers()
            .get("x-ratelimit-remaining")
            .unwrap()
            .to_str()
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(remaining, 0);
    }

    #[tokio::test]
    async fn reset_header_present() {
        let ctx = test_ctx(5);
        let app = test_app(ctx);

        let resp = app.oneshot(ip_request("3.3.3.3")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let reset: u64 = resp
            .headers()
            .get("x-ratelimit-reset")
            .unwrap()
            .to_str()
            .unwrap()
            .parse()
            .unwrap();

        let now = now_epoch();
        assert!(
            reset > now,
            "reset {reset} should be in the future (now {now})"
        );
        assert!(
            reset <= now + WINDOW_SECS,
            "reset {reset} should be within one window of now {now}"
        );
    }

    #[tokio::test]
    async fn api_key_override_uses_custom_limit() {
        let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
        let (_api_key_id, raw_key) = setup_api_key_in_store(&store, Some(1)).await;

        let ctx = RateLimitContext {
            store,
            jwt_config: test_jwt_config(),
            limiter: per_minute(100),
        };

        let app = test_app(ctx.clone());
        let req = Request::builder()
            .uri("/test")
            .header("authorization", format!("Bearer {raw_key}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers()
                .get("x-ratelimit-limit")
                .unwrap()
                .to_str()
                .unwrap(),
            "1"
        );

        // Second request with override=1 should be rejected
        let app = test_app(ctx);
        let req = Request::builder()
            .uri("/test")
            .header("authorization", format!("Bearer {raw_key}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn rate_limited_response_includes_all_headers() {
        let ctx = test_ctx(1);

        let app = test_app(ctx.clone());
        let _ = app.oneshot(ip_request("5.5.5.5")).await;

        let app = test_app(ctx);
        let resp = app.oneshot(ip_request("5.5.5.5")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);

        assert!(resp.headers().contains_key("retry-after"));
        assert!(resp.headers().contains_key("x-ratelimit-limit"));
        assert!(resp.headers().contains_key("x-ratelimit-remaining"));
        assert!(resp.headers().contains_key("x-ratelimit-reset"));

        let remaining: u32 = resp
            .headers()
            .get("x-ratelimit-remaining")
            .unwrap()
            .to_str()
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(remaining, 0);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = serde_json::from_slice(&body).unwrap();
        assert_eq!(json_val["error"]["code"], "RATE_LIMIT_EXCEEDED");
    }

    #[tokio::test]
    async fn different_ips_have_separate_limits() {
        let ctx = test_ctx(1);

        let app = test_app(ctx.clone());
        let resp = app.oneshot(ip_request("10.0.0.1")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let app = test_app(ctx);
        let resp = app.oneshot(ip_request("10.0.0.2")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn allows_requests_within_limit() {
        let ctx = test_ctx(5);
        let app = test_app(ctx);

        let resp = app.oneshot(ip_request("1.2.3.4")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(resp.headers().contains_key("x-ratelimit-limit"));
    }

    #[tokio::test]
    async fn rejects_when_limit_exceeded() {
        let ctx = test_ctx(2);

        let app = test_app(ctx.clone());
        let _ = app.oneshot(ip_request("1.2.3.4")).await;
        let app = test_app(ctx.clone());
        let _ = app.oneshot(ip_request("1.2.3.4")).await;

        let app = test_app(ctx);
        let resp = app.oneshot(ip_request("1.2.3.4")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = serde_json::from_slice(&body).unwrap();
        assert_eq!(json_val["error"]["code"], "RATE_LIMIT_EXCEEDED");
    }

    #[tokio::test]
    async fn includes_retry_after_header() {
        let ctx = test_ctx(1);

        let app = test_app(ctx.clone());
        let _ = app.oneshot(ip_request("1.2.3.4")).await;

        let app = test_app(ctx);
        let resp = app.oneshot(ip_request("1.2.3.4")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
        assert!(resp.headers().contains_key("retry-after"));
    }

    #[tokio::test]
    async fn extracts_ip_from_x_real_ip() {
        let ctx = test_ctx(1);

        let app = test_app(ctx.clone());
        let req = Request::builder()
            .uri("/test")
            .header("x-real-ip", "192.168.1.1")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let app = test_app(ctx);
        let req = Request::builder()
            .uri("/test")
            .header("x-real-ip", "192.168.1.1")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
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

    #[tokio::test]
    async fn general_limiter_allows_more_requests() {
        let ctx = test_ctx(60);

        for _ in 0..10 {
            let app = test_app(ctx.clone());
            let resp = app.oneshot(ip_request("1.2.3.4")).await.unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
        }
    }

    #[tokio::test]
    async fn jwt_cookie_uses_user_bucket() {
        let ctx = test_ctx(1);
        let user_id = Uuid::now_v7();
        let token = AccessToken::for_user(user_id, "cookie-user", false, &ctx.jwt_config).unwrap();

        // Exhaust IP bucket
        let app = test_app(ctx.clone());
        let _ = app.oneshot(ip_request("20.0.0.1")).await;

        // IP exhausted
        let app = test_app(ctx.clone());
        let resp = app.oneshot(ip_request("20.0.0.1")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);

        // JWT in cookie from same IP still works (separate User bucket)
        let app = test_app(ctx);
        let req = Request::builder()
            .uri("/test")
            .header("x-forwarded-for", "20.0.0.1")
            .header("cookie", format!("ironflow_session={}", token.0))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn invalid_bearer_falls_back_to_ip() {
        let ctx = test_ctx(1);

        // First request with garbage Bearer uses IP bucket
        let app = test_app(ctx.clone());
        let req = Request::builder()
            .uri("/test")
            .header("x-forwarded-for", "30.0.0.1")
            .header("authorization", "Bearer not_a_valid_token")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // Second request from same IP is rate limited (same bucket)
        let app = test_app(ctx);
        let req = Request::builder()
            .uri("/test")
            .header("x-forwarded-for", "30.0.0.1")
            .header("authorization", "Bearer not_a_valid_token")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn api_key_override_zero_disables_rate_limiting() {
        let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
        let (_api_key_id, raw_key) = setup_api_key_in_store(&store, Some(0)).await;

        let ctx = RateLimitContext {
            store,
            jwt_config: test_jwt_config(),
            limiter: per_minute(1),
        };

        // override=0 means unlimited: many requests should all pass
        for _ in 0..10 {
            let app = test_app(ctx.clone());
            let req = Request::builder()
                .uri("/test")
                .header("authorization", format!("Bearer {raw_key}"))
                .body(Body::empty())
                .unwrap();
            let resp = app.oneshot(req).await.unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
        }
    }
}
