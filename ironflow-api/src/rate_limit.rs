//! Identity-aware rate limiting middleware.
//!
//! Requests are keyed by authenticated identity (API key ID or user ID) when
//! available, falling back to the client IP address. Each key gets a
//! fixed-window counter that resets every 60 seconds.
//!
//! The client IP is the address of the TCP peer, so the server must be
//! served with `into_make_service_with_connect_info::<SocketAddr>()`.
//! `X-Forwarded-For` and `X-Real-IP` are read only when that peer is one of
//! the [`TrustedProxies`]: any client can write those headers.
//!
//! With [`AccountLimit::ByEmail`], a request whose JSON body names an
//! `email` is also counted against that account, so spreading attempts on
//! one account over many IP addresses does not help.
//!
//! Responses carry standard rate-limit headers:
//! - `X-RateLimit-Limit` -- maximum requests allowed per window
//! - `X-RateLimit-Remaining` -- requests left in the current window
//! - `X-RateLimit-Reset` -- epoch timestamp when the window resets
//!
//! API keys with a `rate_limit_override` use that value instead of the
//! server-wide default.

mod account;
mod client_ip;
#[cfg(test)]
mod tests;

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::body::{Body, to_bytes};
use axum::extract::{ConnectInfo, Request};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use dashmap::DashMap;
use serde_json::json;
use uuid::Uuid;

use ironflow_auth::extractor::{API_KEY_PREFIX, API_KEY_SUFFIX_LEN};
use ironflow_auth::jwt::{AccessToken, JwtConfig};
use ironflow_store::store::Store;

use self::account::{ACCOUNT_BODY_LIMIT, account_key};
use self::client_ip::client_ip;

pub use self::account::AccountLimit;
pub use self::client_ip::{InvalidTrustedProxy, TrustedProxies};

/// Counters kept before expired windows are swept. Account keys are chosen
/// by the caller, so the map must not grow without bound.
const SWEEP_THRESHOLD: usize = 10_000;

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
/// let account_key = RateLimitKey::Account("alice@ironflow.dev".to_string());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RateLimitKey {
    /// Authenticated via API key.
    ApiKey(Uuid),
    /// Authenticated via JWT.
    User(Uuid),
    /// Unauthenticated, identified by client IP.
    Ip(IpAddr),
    /// The account a credential request targets, by normalized email.
    Account(String),
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
/// use ironflow_api::rate_limit::{AccountLimit, RateLimitContext, TrustedProxies, per_minute};
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
///     trusted_proxies: TrustedProxies::default(),
///     account_limit: AccountLimit::Off,
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
    /// Proxies allowed to report the client IP through forwarding headers.
    pub trusted_proxies: TrustedProxies,
    /// Whether requests are also counted against the account they target.
    pub account_limit: AccountLimit,
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

    if limiter.counters.len() >= SWEEP_THRESHOLD && !limiter.counters.contains_key(&key) {
        limiter
            .counters
            .retain(|_, entry| entry.window_start.load(Ordering::Acquire) + WINDOW_SECS > now);
    }

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
/// - Fallback -> `RateLimitKey::Ip(addr)`, the TCP peer or, behind a
///   [trusted proxy](TrustedProxies), the address it forwarded
///
/// API keys with a `rate_limit_override` use that value instead of
/// the default burst. With [`AccountLimit::ByEmail`], the targeted account
/// is counted too, at the default burst; a body over 64 KiB is answered
/// with `413` before reaching the handler.
pub async fn rate_limit(mut req: Request, next: Next) -> Response {
    let ctx = req.extensions_mut().remove::<RateLimitContext>();
    let Some(ctx) = ctx else {
        return next.run(req).await;
    };
    let peer = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(addr)| addr.ip());
    let (key, override_limit) = extract_rate_limit_key(req.headers(), peer, &ctx).await;
    let limit = override_limit.unwrap_or(ctx.limiter.burst);

    if limit == 0 {
        return next.run(req).await;
    }

    let caller = check_rate_limit(&ctx.limiter, key, limit);
    if !caller.allowed {
        return too_many_requests(&caller);
    }

    let (req, result) = match ctx.account_limit {
        AccountLimit::Off => (req, caller),
        AccountLimit::ByEmail => {
            let (parts, body) = req.into_parts();
            let Ok(bytes) = to_bytes(body, ACCOUNT_BODY_LIMIT).await else {
                return payload_too_large();
            };
            let account = account_key(&bytes);
            let req = Request::from_parts(parts, Body::from(bytes));
            match account {
                None => (req, caller),
                Some(key) => {
                    let account = check_rate_limit(&ctx.limiter, key, ctx.limiter.burst);
                    if !account.allowed {
                        return too_many_requests(&account);
                    }
                    let tightest = if account.remaining < caller.remaining {
                        account
                    } else {
                        caller
                    };
                    (req, tightest)
                }
            }
        }
    };

    let mut resp = next.run(req).await;
    insert_rate_limit_headers(resp.headers_mut(), &result);
    resp
}

fn too_many_requests(result: &RateLimitResult) -> Response {
    let retry_after = result.reset.saturating_sub(now_epoch()).max(1);

    let body = json!({
        "error": {
            "code": "RATE_LIMIT_EXCEEDED",
            "message": "Too many requests, please try again later",
            "retry_after_secs": retry_after,
        }
    });

    let mut resp = (StatusCode::TOO_MANY_REQUESTS, Json(body)).into_response();
    resp.headers_mut()
        .insert("retry-after", HeaderValue::from(retry_after));
    insert_rate_limit_headers(resp.headers_mut(), result);
    resp
}

fn payload_too_large() -> Response {
    let body = json!({
        "error": {
            "code": "PAYLOAD_TOO_LARGE",
            "message": "Request body too large or unreadable",
        }
    });
    (StatusCode::PAYLOAD_TOO_LARGE, Json(body)).into_response()
}

fn insert_rate_limit_headers(headers: &mut HeaderMap, result: &RateLimitResult) {
    headers.insert("x-ratelimit-limit", HeaderValue::from(result.limit));
    headers.insert("x-ratelimit-remaining", HeaderValue::from(result.remaining));
    headers.insert("x-ratelimit-reset", HeaderValue::from(result.reset));
}

/// Extract the rate limit key and optional override from the request.
///
/// Attempts lightweight identity extraction without full auth verification:
/// - API key prefix lookup (cheap index scan, no argon2)
/// - JWT decode (local crypto, no DB)
/// - Fallback to the client IP
async fn extract_rate_limit_key(
    headers: &HeaderMap,
    peer: Option<IpAddr>,
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

    let ip = client_ip(peer, headers, &ctx.trusted_proxies);
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
