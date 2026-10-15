//! Identity-aware rate limiting middleware.
//!
//! Requests are keyed by authenticated identity (API key ID or user ID) when
//! available, falling back to the client IP address. Each key gets a
//! fixed-window counter that resets every 60 seconds.
//!
//! The client IP is the address of the TCP peer, so the server must be
//! served with `into_make_service_with_connect_info::<SocketAddr>()`.
//! The configured forwarding header ([`TrustedProxies::header`]) is read
//! only when that peer is one of the [`TrustedProxies`]: any client can
//! write it. Without a peer address the IP limit is skipped.
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
//! A request is counted under an API key only once its secret is verified,
//! and the key's `rate_limit_override` replaces the server-wide default only
//! when the key is owned by an admin. Secrets not yet verified are checked
//! at most `KEY_CHECKS_PER_MINUTE` times per client and key each minute.

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
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde_json::json;
use tokio::sync::Mutex;
use tracing::warn;
use uuid::Uuid;

use ironflow_auth::cookies::AUTH_COOKIE_NAME;
use ironflow_auth::extractor::{API_KEY_PREFIX, API_KEY_SUFFIX_LEN};
use ironflow_auth::jwt::{AccessToken, JwtConfig, token_hash};
use ironflow_auth::password;
use ironflow_store::store::Store;

use self::account::{ACCOUNT_BODY_LIMIT, account_key};
use self::client_ip::client_ip;

pub use self::account::AccountLimit;
pub use self::client_ip::{
    ClientIpHeader, InvalidClientIpHeader, InvalidTrustedProxy, TrustedProxies,
};

/// Counters kept before expired windows are swept. Account keys are chosen
/// by the caller, so the map must not grow without bound.
const SWEEP_THRESHOLD: usize = 10_000;

/// Seconds a checked API key token keeps its verdict. Argon2 is too slow to
/// run on every request, and a minute bounds how long a deactivated key or a
/// demoted owner keeps its bucket.
const VERIFIED_KEY_TTL_SECS: u64 = 60;

/// Argon2 verifications allowed per client and per API key each minute for
/// tokens not in the cache. A legitimate client needs one per key every
/// `VERIFIED_KEY_TTL_SECS`; more is someone guessing secrets, refused before
/// Argon2 so the limiter is not a CPU amplifier (a fresh secret on every
/// request misses the cache every time).
const KEY_CHECKS_PER_MINUTE: u32 = 10;

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
/// let account_key = RateLimitKey::Account {
///     route: "/sign-in".to_string(),
///     email: "alice@ironflow.dev".to_string(),
/// };
/// let key_check = RateLimitKey::KeyCheck {
///     ip: Some(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))),
///     api_key: Uuid::nil(),
/// };
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RateLimitKey {
    /// Authenticated via API key.
    ApiKey(Uuid),
    /// Authenticated via JWT.
    User(Uuid),
    /// Unauthenticated, identified by client IP.
    Ip(IpAddr),
    /// The account a credential request targets on one route.
    Account {
        /// Path of the credential route, so sign-up and sign-in are counted
        /// apart.
        route: String,
        /// Normalized email of the account.
        email: String,
    },
    /// Argon2 verifications of an API key secret not yet in the cache, by
    /// client IP, or by key alone when the server has no peer address.
    ///
    /// Keyed on both so an attacker cannot exhaust the budget of a victim key
    /// from another address, and one NAT address using many keys is not
    /// throttled.
    KeyCheck {
        /// Client IP, `None` when the server has no peer address.
        ip: Option<IpAddr>,
        /// ID of the API key whose secret is checked.
        api_key: Uuid,
    },
}

struct WindowEntry {
    count: AtomicU32,
    window_start: AtomicU64,
}

/// The verdict on one API key token, kept `VERIFIED_KEY_TTL_SECS`.
struct CachedVerification {
    /// Key ID and the override it is granted; `None` when the secret was
    /// checked by Argon2 and is wrong, so a replayed bad token does not cost
    /// one Argon2 per request.
    identity: Option<(Uuid, Option<u32>)>,
    valid_until: u64,
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
    /// Checked API key tokens, by `token_hash`: the raw token is never kept.
    verified_keys: Arc<DashMap<String, CachedVerification>>,
    /// Verifications in flight, by `token_hash`: concurrent requests with the
    /// same token wait for the first verdict instead of each spending the
    /// verification budget.
    pending_checks: Arc<DashMap<String, Arc<Mutex<()>>>>,
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
        verified_keys: Arc::new(DashMap::new()),
        pending_checks: Arc::new(DashMap::new()),
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
/// - API key (`irfl_*` Bearer token whose secret verifies) ->
///   `RateLimitKey::ApiKey(id)`
/// - JWT (Bearer token or auth cookie) -> `RateLimitKey::User(id)`
/// - Fallback -> `RateLimitKey::Ip(addr)`, the TCP peer or, behind a
///   [trusted proxy](TrustedProxies), the address it forwarded. Without a
///   peer address no caller limit applies.
///
/// A verified API key owned by an admin uses its `rate_limit_override`
/// instead of the default burst. Once a client has spent its budget of
/// API key secret verifications for a key, its next unverified secret is
/// answered with `429` before Argon2 runs. With [`AccountLimit::ByEmail`],
/// the targeted account is counted too, per route, at the default burst; a
/// body over 64 KiB is answered with `413` before reaching the handler.
pub async fn rate_limit(mut req: Request, next: Next) -> Response {
    let ctx = req.extensions_mut().remove::<RateLimitContext>();
    let Some(ctx) = ctx else {
        return next.run(req).await;
    };
    let peer = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(addr)| addr.ip());
    let identified = match extract_rate_limit_key(req.headers(), peer, &ctx).await {
        Ok(identified) => identified,
        Err(result) => return too_many_requests(&result),
    };

    let caller = match identified {
        Some((key, override_limit)) => {
            let limit = override_limit.unwrap_or(ctx.limiter.burst);
            if limit == 0 {
                return next.run(req).await;
            }
            let result = check_rate_limit(&ctx.limiter, key, limit);
            if !result.allowed {
                return too_many_requests(&result);
            }
            Some(result)
        }
        None => None,
    };

    let (req, result) = match ctx.account_limit {
        AccountLimit::Off => (req, caller),
        AccountLimit::ByEmail => {
            let (parts, body) = req.into_parts();
            let Ok(bytes) = to_bytes(body, ACCOUNT_BODY_LIMIT).await else {
                return payload_too_large();
            };
            // The layer only wraps matched static routes (`/sign-up`,
            // `/sign-in`), so a client cannot mint new buckets through the
            // path.
            let account = account_key(parts.uri.path(), &bytes);
            let req = Request::from_parts(parts, Body::from(bytes));
            match account {
                None => (req, caller),
                Some(key) => {
                    let account = check_rate_limit(&ctx.limiter, key, ctx.limiter.burst);
                    if !account.allowed {
                        return too_many_requests(&account);
                    }
                    let tightest = match caller {
                        Some(caller) if caller.remaining <= account.remaining => caller,
                        _ => account,
                    };
                    (req, Some(tightest))
                }
            }
        }
    };

    let mut resp = next.run(req).await;
    if let Some(result) = &result {
        insert_rate_limit_headers(resp.headers_mut(), result);
    }
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
/// Attempts lightweight identity extraction:
/// - API key prefix lookup plus an Argon2 verification of the secret, cached
///   `VERIFIED_KEY_TTL_SECS` and budgeted by `KEY_CHECKS_PER_MINUTE`
/// - JWT decode (local crypto, no DB)
/// - Fallback to the client IP
///
/// `Ok(None)` when no identity matched and there is no client IP; `Err`
/// when the API key verification budget is spent.
async fn extract_rate_limit_key(
    headers: &HeaderMap,
    peer: Option<IpAddr>,
    ctx: &RateLimitContext,
) -> Result<Option<(RateLimitKey, Option<u32>)>, RateLimitResult> {
    let ip = client_ip(peer, headers, &ctx.trusted_proxies);
    let bearer = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|s| s.to_string());

    if let Some(ref token) = bearer {
        if token.starts_with(API_KEY_PREFIX) {
            if let Some(found) = try_api_key_identity(token, ip, ctx).await? {
                return Ok(Some(found));
            }
        } else if let Some(key) = try_jwt_identity(token, ctx) {
            return Ok(Some((key, None)));
        }
    }

    // Try JWT from the dashboard auth cookie
    if let Some(cookie_header) = headers.get("cookie").and_then(|v| v.to_str().ok()) {
        for part in cookie_header.split(';') {
            let cookie = part.trim().strip_prefix(AUTH_COOKIE_NAME);
            if let Some(value) = cookie.and_then(|rest| rest.strip_prefix('='))
                && let Some(key) = try_jwt_identity(value, ctx)
            {
                return Ok(Some((key, None)));
            }
        }
    }

    Ok(ip.map(|ip| (RateLimitKey::Ip(ip), None)))
}

/// Outcome of checking an API key token against the store.
enum Verification {
    /// The secret matches an active, unexpired key.
    Valid {
        id: Uuid,
        /// The key's override, kept only when its owner is an admin.
        rate_limit_override: Option<u32>,
        expires_at: Option<DateTime<Utc>>,
    },
    /// No such key, or an inactive or expired key. Cheap to tell again, so
    /// not cached: caching it would let a client grow the cache with fresh
    /// random tokens.
    Unknown,
    /// The secret does not match the key, as checked by Argon2.
    WrongSecret,
    /// The store or Argon2 failed: not a verdict, so not cached.
    Unavailable,
    /// The verification budget of this client and key is spent.
    Throttled(RateLimitResult),
}

/// The API key bucket of `token`, once its secret is verified.
///
/// `Ok(None)` when the token is not a usable key, so the request falls back
/// to the cookie and IP buckets. `Err` when the verification budget is
/// spent: the request must be answered `429`.
async fn try_api_key_identity(
    token: &str,
    ip: Option<IpAddr>,
    ctx: &RateLimitContext,
) -> Result<Option<(RateLimitKey, Option<u32>)>, RateLimitResult> {
    let now = now_epoch();
    let hash = token_hash(token);
    let as_key = |(id, limit): (Uuid, Option<u32>)| (RateLimitKey::ApiKey(id), limit);

    if let Some(identity) = cached_verdict(ctx, &hash, now) {
        return Ok(identity.map(as_key));
    }

    // The map guard is dropped at the end of the statement, before the await.
    let lock = Arc::clone(
        ctx.limiter
            .pending_checks
            .entry(hash.clone())
            .or_default()
            .value(),
    );
    let _pending = PendingCheck {
        pending_checks: &ctx.limiter.pending_checks,
        hash: &hash,
        lock: Arc::clone(&lock),
    };
    let _turn = lock.lock().await;
    // A concurrent request with the same token may have settled it meanwhile.
    if let Some(identity) = cached_verdict(ctx, &hash, now) {
        return Ok(identity.map(as_key));
    }

    let ttl_end = now + VERIFIED_KEY_TTL_SECS;
    let (identity, valid_until) = match verify_api_key(token, ip, ctx).await {
        Verification::Valid {
            id,
            rate_limit_override,
            expires_at,
        } => {
            let expiry = expires_at.map_or(u64::MAX, |at| at.timestamp().max(0) as u64);
            (Some((id, rate_limit_override)), ttl_end.min(expiry))
        }
        Verification::WrongSecret => (None, ttl_end),
        Verification::Unknown | Verification::Unavailable => return Ok(None),
        Verification::Throttled(result) => return Err(result),
    };

    let verified_keys = &ctx.limiter.verified_keys;
    if verified_keys.len() >= SWEEP_THRESHOLD && !verified_keys.contains_key(&hash) {
        verified_keys.retain(|_, cached| cached.valid_until > now);
    }
    let verdict = CachedVerification {
        identity,
        valid_until,
    };
    verified_keys.insert(hash.clone(), verdict);
    Ok(identity.map(as_key))
}

/// The unexpired cached verdict on the token hashed to `hash`.
fn cached_verdict(
    ctx: &RateLimitContext,
    hash: &str,
    now: u64,
) -> Option<Option<(Uuid, Option<u32>)>> {
    // The map guard is dropped inside `map`, before any await or insert.
    ctx.limiter
        .verified_keys
        .get(hash)
        .filter(|cached| cached.valid_until > now)
        .map(|cached| cached.identity)
}

/// Removes the in-flight entry of a verification when it ends, even when the
/// request is cancelled mid-await.
struct PendingCheck<'a> {
    pending_checks: &'a DashMap<String, Arc<Mutex<()>>>,
    hash: &'a str,
    lock: Arc<Mutex<()>>,
}

impl Drop for PendingCheck<'_> {
    fn drop(&mut self) {
        // A later request may have registered a new lock once this one was
        // removed: only remove our own.
        self.pending_checks
            .remove_if(self.hash, |_, lock| Arc::ptr_eq(lock, &self.lock));
    }
}

/// Check `token` the way the `ApiKeyAuth` extractor does, without touching
/// the key's `last_used_at`.
async fn verify_api_key(token: &str, ip: Option<IpAddr>, ctx: &RateLimitContext) -> Verification {
    let suffix_len = (token.len() - API_KEY_PREFIX.len()).min(API_KEY_SUFFIX_LEN);
    let prefix = &token[..API_KEY_PREFIX.len() + suffix_len];

    let api_key = match ctx.store.find_api_key_by_prefix(prefix).await {
        Ok(Some(api_key)) => api_key,
        Ok(None) => return Verification::Unknown,
        Err(err) => {
            warn!(error = %err, "rate limiter could not look up an API key");
            return Verification::Unavailable;
        }
    };

    if !api_key.is_active || api_key.expires_at.is_some_and(|at| at <= Utc::now()) {
        return Verification::Unknown;
    }

    // Only reached on a cache miss for an existing, active, unexpired key, so
    // each Argon2 below is paid for out of this budget.
    let check = check_rate_limit(
        &ctx.limiter,
        RateLimitKey::KeyCheck {
            ip,
            api_key: api_key.id,
        },
        KEY_CHECKS_PER_MINUTE,
    );
    if !check.allowed {
        return Verification::Throttled(check);
    }

    match password::verify(token, &api_key.key_hash) {
        Ok(true) => {}
        Ok(false) => return Verification::WrongSecret,
        Err(err) => {
            warn!(error = %err, "rate limiter could not verify an API key");
            return Verification::Unavailable;
        }
    }

    // Only admins may set an override; one a member set before that rule is
    // ignored.
    let rate_limit_override = match ctx.store.find_user_by_id(api_key.user_id).await {
        Ok(Some(owner)) if owner.is_admin => api_key.rate_limit_override,
        Ok(_) => None,
        Err(err) => {
            warn!(error = %err, "rate limiter could not look up an API key owner");
            return Verification::Unavailable;
        }
    };

    Verification::Valid {
        id: api_key.id,
        rate_limit_override,
        expires_at: api_key.expires_at,
    }
}

fn try_jwt_identity(token: &str, ctx: &RateLimitContext) -> Option<RateLimitKey> {
    let claims = AccessToken::decode(token, &ctx.jwt_config).ok()?;
    Some(RateLimitKey::User(claims.user_id))
}
