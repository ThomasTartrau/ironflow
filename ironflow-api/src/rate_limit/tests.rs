//! Tests of the rate limit middleware, through a real axum router.
//!
//! Shared helpers live here; the tests are split by concern.

mod account;
mod forwarding;
mod identity;

use std::net::SocketAddr;

use axum::Extension;
use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::Request;
use axum::middleware as axum_mw;
use axum::routing::{get, post};
use ironflow_auth::extractor::API_KEY_SUFFIX_LEN;
use ironflow_auth::password;
use ironflow_store::entities::{ApiKeyScope, NewApiKey, NewUser};
use ironflow_store::memory::InMemoryStore;

use super::*;

async fn ok_handler() -> &'static str {
    "ok"
}

async fn echo_handler(body: String) -> String {
    body
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

fn ctx_with_store(store: Arc<dyn Store>, burst: u32) -> RateLimitContext {
    RateLimitContext {
        store,
        jwt_config: test_jwt_config(),
        limiter: per_minute(burst),
        trusted_proxies: TrustedProxies::default(),
        account_limit: AccountLimit::Off,
    }
}

fn test_ctx(burst: u32) -> RateLimitContext {
    ctx_with_store(Arc::new(InMemoryStore::new()), burst)
}

fn test_app(ctx: RateLimitContext) -> Router {
    Router::new()
        .route("/test", get(ok_handler))
        .route("/echo", post(echo_handler))
        .layer(axum_mw::from_fn(rate_limit))
        .layer(Extension(ctx))
}

/// What a real server attaches from the TCP socket.
fn with_peer(mut req: Request<Body>, peer: &str) -> Request<Body> {
    let addr = SocketAddr::new(peer.parse().unwrap(), 40000);
    req.extensions_mut().insert(ConnectInfo(addr));
    req
}

fn ip_request(ip: &str) -> Request<Body> {
    with_peer(
        Request::builder().uri("/test").body(Body::empty()).unwrap(),
        ip,
    )
}

fn header_request(peer: &str, name: &str, value: &str) -> Request<Body> {
    with_peer(
        Request::builder()
            .uri("/test")
            .header(name, value)
            .body(Body::empty())
            .unwrap(),
        peer,
    )
}

fn email_request(peer: &str, body: &str) -> Request<Body> {
    with_peer(
        Request::builder()
            .method("POST")
            .uri("/echo")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
        peer,
    )
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
    let prefix = &raw_key[..API_KEY_PREFIX.len() + API_KEY_SUFFIX_LEN];

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
