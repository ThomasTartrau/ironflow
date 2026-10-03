//! Integration test: sessions are revoked server-side.
//!
//! A refresh token works once, sign-out kills the access token it was called
//! with, and a demoted admin loses admin access on the next request. Each test
//! drives the full router, as a browser would, through the auth cookies.

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Method, Request, Response, StatusCode};
use ironflow_api::routes::{RouterConfig, create_router};
use ironflow_api::state::AppState;
use ironflow_auth::cookies::{AUTH_COOKIE_NAME, REFRESH_COOKIE_NAME};
use ironflow_auth::jwt::JwtConfig;
use ironflow_auth::password;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_engine::engine::Engine;
use ironflow_engine::notify::Event;
use ironflow_store::entities::{NewUser, User};
use ironflow_store::memory::InMemoryStore;
use ironflow_store::store::Store;
use serde_json::json;
use tokio::sync::broadcast;
use tokio::time::timeout;
use tower::ServiceExt;

const TEST_TIMEOUT: Duration = Duration::from_secs(10);
const EMAIL: &str = "alice@example.com";
const PASSWORD: &str = "password123";

fn test_app() -> (Router, Arc<dyn Store>) {
    let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
    let provider = Arc::new(ClaudeCodeProvider::new());
    let engine = Engine::new(store.clone(), provider);
    let jwt_config = Arc::new(JwtConfig {
        secret: "test-secret-for-session-revocation".to_string(),
        access_token_ttl_secs: 900,
        refresh_token_ttl_secs: 604800,
        cookie_domain: None,
        cookie_secure: false,
    });
    let (event_sender, _) = broadcast::channel::<Event>(1);
    let state = AppState::new(
        store.clone(),
        Arc::new(engine),
        jwt_config,
        "test-worker-token".to_string(),
        event_sender,
    );
    // The limiter keys on the peer address, which `oneshot` does not provide.
    let config = RouterConfig {
        rate_limit_auth: None,
        rate_limit_general: None,
        ..RouterConfig::default()
    };
    (create_router(state, config), store)
}

async fn create_user(store: &Arc<dyn Store>, is_admin: bool) -> User {
    let hash = password::hash(PASSWORD).expect("hash");
    store
        .create_user(NewUser {
            email: EMAIL.to_string(),
            username: "alice".to_string(),
            password_hash: hash,
            is_admin: Some(is_admin),
        })
        .await
        .expect("create user")
}

/// Value of the cookie `name` set by `headers`.
fn set_cookie(headers: &HeaderMap, name: &str) -> String {
    let prefix = format!("{name}=");
    headers
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find_map(|c| c.strip_prefix(prefix.as_str()))
        .and_then(|rest| rest.split(';').next())
        .expect("cookie not set")
        .to_string()
}

async fn send(app: &Router, method: Method, uri: &str, cookie: Option<String>) -> Response<Body> {
    let mut req = Request::builder().method(method).uri(uri);
    if let Some(cookie) = cookie {
        req = req.header("cookie", cookie);
    }
    app.clone()
        .oneshot(req.body(Body::empty()).expect("build request"))
        .await
        .expect("request")
}

/// Sign in through the API and return the `Set-Cookie` headers.
async fn sign_in(app: &Router) -> HeaderMap {
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/v1/auth/sign-in")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({ "email": EMAIL, "password": PASSWORD }).to_string(),
        ))
        .expect("build request");
    let resp = app.clone().oneshot(req).await.expect("request");
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    resp.headers().clone()
}

#[tokio::test]
async fn refresh_token_replayed_twice_returns_401() {
    timeout(TEST_TIMEOUT, async {
        let (app, store) = test_app();
        create_user(&store, false).await;
        let headers = sign_in(&app).await;
        let refresh = format!(
            "{REFRESH_COOKIE_NAME}={}",
            set_cookie(&headers, REFRESH_COOKIE_NAME)
        );

        let first = send(
            &app,
            Method::POST,
            "/api/v1/auth/refresh",
            Some(refresh.clone()),
        )
        .await;
        assert_eq!(first.status(), StatusCode::NO_CONTENT);

        let replay = send(&app, Method::POST, "/api/v1/auth/refresh", Some(refresh)).await;
        assert_eq!(replay.status(), StatusCode::UNAUTHORIZED);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn access_token_reused_after_sign_out_returns_401() {
    timeout(TEST_TIMEOUT, async {
        let (app, store) = test_app();
        create_user(&store, false).await;
        let headers = sign_in(&app).await;
        let access = format!(
            "{AUTH_COOKIE_NAME}={}",
            set_cookie(&headers, AUTH_COOKIE_NAME)
        );

        let before = send(&app, Method::GET, "/api/v1/runs", Some(access.clone())).await;
        assert_eq!(before.status(), StatusCode::OK);

        let signed_out = send(
            &app,
            Method::POST,
            "/api/v1/auth/sign-out",
            Some(access.clone()),
        )
        .await;
        assert_eq!(signed_out.status(), StatusCode::OK);

        // The browser dropped the cookie, but a copy of the token kept
        // elsewhere must not work either.
        let after = send(&app, Method::GET, "/api/v1/runs", Some(access)).await;
        assert_eq!(after.status(), StatusCode::UNAUTHORIZED);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn demoted_admin_keeps_no_admin_access() {
    timeout(TEST_TIMEOUT, async {
        let (app, store) = test_app();
        let admin = create_user(&store, true).await;
        let headers = sign_in(&app).await;
        let access = format!(
            "{AUTH_COOKIE_NAME}={}",
            set_cookie(&headers, AUTH_COOKIE_NAME)
        );

        let before = send(&app, Method::GET, "/api/v1/users", Some(access.clone())).await;
        assert_eq!(before.status(), StatusCode::OK);

        store
            .update_user_role(admin.id, false)
            .await
            .expect("demote user");

        // The token still claims is_admin = true and has not expired.
        let after = send(&app, Method::GET, "/api/v1/users", Some(access)).await;
        assert_eq!(after.status(), StatusCode::UNAUTHORIZED);
    })
    .await
    .expect("test timed out");
}
