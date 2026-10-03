//! Integration test: the auth credential routes resist the abuses found in
//! pentest AUTH-VULN-03, 08 and 09.
//!
//! Every test drives the full router built by [`create_router`]. The peer
//! address a real server would get from the TCP socket is attached to each
//! request as [`ConnectInfo`], since `oneshot` has no socket.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, Response, StatusCode};
use http_body_util::BodyExt;
use ironflow_api::routes::{RouterConfig, create_router};
use ironflow_api::state::AppState;
use ironflow_auth::jwt::{AccessToken, JwtConfig};
use ironflow_auth::password;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_engine::engine::Engine;
use ironflow_engine::notify::Event;
use ironflow_store::entities::{NewUser, User};
use ironflow_store::memory::InMemoryStore;
use ironflow_store::store::Store;
use serde_json::{Value, from_slice, json};
use tokio::sync::broadcast;
use tokio::time::timeout;
use tower::ServiceExt;

const TEST_TIMEOUT: Duration = Duration::from_secs(30);
const PASSWORD: &str = "correct horse battery staple";
const OTHER_PASSWORD: &str = "Tq9!vR2#mZ7$ wq";

/// What a client can observe of a response: status, body and session cookies.
#[derive(Debug, PartialEq)]
struct Answer {
    status: StatusCode,
    body: Value,
    set_cookies: usize,
}

impl Answer {
    async fn read(resp: Response<Body>) -> Self {
        let status = resp.status();
        let set_cookies = resp.headers().get_all("set-cookie").iter().count();
        let bytes = resp.into_body().collect().await.expect("body").to_bytes();
        let body = if bytes.is_empty() {
            Value::Null
        } else {
            from_slice(&bytes).expect("json body")
        };
        Self {
            status,
            body,
            set_cookies,
        }
    }
}

struct Harness {
    app: Router,
    store: Arc<dyn Store>,
    jwt_config: Arc<JwtConfig>,
}

impl Harness {
    fn new(config: RouterConfig) -> Self {
        let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
        let provider = Arc::new(ClaudeCodeProvider::new());
        let engine = Engine::new(store.clone(), provider);
        let jwt_config = Arc::new(JwtConfig {
            secret: "test-secret-for-auth-hardening".to_string(),
            access_token_ttl_secs: 900,
            refresh_token_ttl_secs: 604800,
            cookie_domain: None,
            cookie_secure: false,
        });
        let (event_sender, _) = broadcast::channel::<Event>(1);
        let state = AppState::new(
            store.clone(),
            Arc::new(engine),
            jwt_config.clone(),
            "test-worker-token".to_string(),
            event_sender,
        );
        Self {
            app: create_router(state, config),
            store,
            jwt_config,
        }
    }

    /// A stored user and a `Bearer` header for it.
    async fn user(&self, username: &str, password: &str, is_admin: bool) -> (User, String) {
        let user = self
            .store
            .create_user(NewUser {
                email: format!("{username}@ironflow.dev"),
                username: username.to_string(),
                password_hash: password::hash(password).expect("hash"),
                is_admin: Some(is_admin),
            })
            .await
            .expect("create user");
        let token = AccessToken::for_user(user.id, &user.username, is_admin, &self.jwt_config)
            .expect("token");
        (user, format!("Bearer {}", token.0))
    }

    /// The status of a sign-in attempt, each from its own address so the IP
    /// bucket stays out of the way.
    async fn sign_in(&self, email: &str, password: &str) -> StatusCode {
        let n = SIGN_IN_PEER.fetch_add(1, Ordering::Relaxed);
        let req = post(
            "/api/v1/auth/sign-in",
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 0, 2, n)), 40000),
            None,
            json!({"email": email, "password": password}),
        );
        self.app
            .clone()
            .oneshot(req)
            .await
            .expect("response")
            .status()
    }
}

static SIGN_IN_PEER: AtomicU8 = AtomicU8::new(1);

fn test_app(config: RouterConfig) -> Router {
    Harness::new(config).app
}

fn authed(method: &str, path: &str, auth: &str, body: Value) -> Request<Body> {
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json")
        .header("authorization", auth)
        .body(Body::from(body.to_string()))
        .expect("request");
    req.extensions_mut()
        .insert(ConnectInfo(peer("198.51.100.200")));
    req
}

fn post(path: &str, peer: SocketAddr, forwarded_for: Option<&str>, body: Value) -> Request<Body> {
    let mut builder = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json");
    if let Some(value) = forwarded_for {
        builder = builder.header("x-forwarded-for", value);
    }
    let mut req = builder.body(Body::from(body.to_string())).expect("request");
    req.extensions_mut().insert(ConnectInfo(peer));
    req
}

fn peer(ip: &str) -> SocketAddr {
    SocketAddr::new(ip.parse().expect("ip"), 40000)
}

#[tokio::test]
async fn rotating_x_forwarded_for_does_not_bypass_sign_in_rate_limit() {
    timeout(TEST_TIMEOUT, async {
        let app = test_app(RouterConfig::default());
        let mut statuses = Vec::new();
        for i in 1..=20 {
            let req = post(
                "/api/v1/auth/sign-in",
                peer("203.0.113.7"),
                Some(&format!("9.9.9.{i}")),
                json!({"email": format!("user{i}@x.dev"), "password": "wrong"}),
            );
            let resp = app.clone().oneshot(req).await.expect("response");
            statuses.push(resp.status());
        }
        assert!(
            statuses.contains(&StatusCode::TOO_MANY_REQUESTS),
            "20 sign-in attempts from one peer with a rotating X-Forwarded-For \
             were never rate limited: {statuses:?}"
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn sign_in_attempts_on_one_account_from_many_addresses_are_rate_limited() {
    timeout(TEST_TIMEOUT, async {
        let app = test_app(RouterConfig::default());
        let mut statuses = Vec::new();
        for i in 1..=20 {
            // Casing varies too: the account bucket normalizes the email.
            let email = if i % 2 == 0 { "x@x.dev" } else { "X@X.DEV" };
            let req = post(
                "/api/v1/auth/sign-in",
                peer(&format!("198.51.100.{i}")),
                None,
                json!({"email": email, "password": "wrong"}),
            );
            let resp = app.clone().oneshot(req).await.expect("response");
            statuses.push(resp.status());
        }
        assert!(
            statuses.contains(&StatusCode::TOO_MANY_REQUESTS),
            "20 sign-in attempts on one account from 20 addresses were never \
             rate limited: {statuses:?}"
        );
    })
    .await
    .expect("test timed out");
}

#[cfg(feature = "sign-up")]
#[tokio::test]
async fn sign_up_with_an_existing_email_answers_like_a_new_email() {
    timeout(TEST_TIMEOUT, async {
        let app = test_app(RouterConfig::default());

        let first = post(
            "/api/v1/auth/sign-up",
            peer("198.51.100.1"),
            None,
            json!({"email": "alice@ironflow.dev", "username": "alice", "password": PASSWORD}),
        );
        let first = app.clone().oneshot(first).await.expect("response");
        assert!(first.status().is_success(), "first: {}", first.status());

        let fresh = post(
            "/api/v1/auth/sign-up",
            peer("198.51.100.2"),
            None,
            json!({"email": "carol@ironflow.dev", "username": "carol", "password": PASSWORD}),
        );
        let fresh = Answer::read(app.clone().oneshot(fresh).await.expect("response")).await;

        let taken = post(
            "/api/v1/auth/sign-up",
            peer("198.51.100.3"),
            None,
            json!({"email": "alice@ironflow.dev", "username": "mallory", "password": OTHER_PASSWORD}),
        );
        let taken = Answer::read(app.clone().oneshot(taken).await.expect("response")).await;

        assert_eq!(
            taken, fresh,
            "an existing email must not be distinguishable from a new one"
        );
        assert_eq!(fresh.set_cookies, 0, "sign-up must not open a session");
    })
    .await
    .expect("test timed out");
}

#[cfg(feature = "sign-up")]
#[tokio::test]
async fn sign_up_with_an_existing_email_does_not_touch_that_account() {
    timeout(TEST_TIMEOUT, async {
        let app = test_app(RouterConfig::default());
        for (ip, username, password) in [
            ("198.51.100.1", "alice", PASSWORD),
            ("198.51.100.2", "mallory", OTHER_PASSWORD),
        ] {
            let req = post(
                "/api/v1/auth/sign-up",
                peer(ip),
                None,
                json!({"email": "alice@ironflow.dev", "username": username, "password": password}),
            );
            let resp = app.clone().oneshot(req).await.expect("response");
            assert_eq!(resp.status(), StatusCode::NO_CONTENT);
        }

        let sign_in = |password: &str, ip: &str| {
            post(
                "/api/v1/auth/sign-in",
                peer(ip),
                None,
                json!({"email": "alice@ironflow.dev", "password": password}),
            )
        };
        let owner = app
            .clone()
            .oneshot(sign_in(PASSWORD, "198.51.100.4"))
            .await
            .expect("response");
        assert_eq!(owner.status(), StatusCode::NO_CONTENT);
        let intruder = app
            .clone()
            .oneshot(sign_in(OTHER_PASSWORD, "198.51.100.5"))
            .await
            .expect("response");
        assert_eq!(intruder.status(), StatusCode::UNAUTHORIZED);
    })
    .await
    .expect("test timed out");
}

#[cfg(feature = "sign-up")]
#[tokio::test]
async fn sign_up_with_a_taken_username_is_refused_whatever_the_email() {
    timeout(TEST_TIMEOUT, async {
        let app = test_app(RouterConfig::default());
        let seed = post(
            "/api/v1/auth/sign-up",
            peer("198.51.100.1"),
            None,
            json!({"email": "alice@ironflow.dev", "username": "alice", "password": PASSWORD}),
        );
        let seed = app.clone().oneshot(seed).await.expect("response");
        assert_eq!(seed.status(), StatusCode::NO_CONTENT);

        // Username taken: the same 409 for an existing and a new email, so the
        // answer says nothing about the email.
        let mut answers = Vec::new();
        for (ip, email) in [
            ("198.51.100.2", "alice@ironflow.dev"),
            ("198.51.100.3", "nobody@ironflow.dev"),
        ] {
            let req = post(
                "/api/v1/auth/sign-up",
                peer(ip),
                None,
                json!({"email": email, "username": "alice", "password": OTHER_PASSWORD}),
            );
            answers.push(Answer::read(app.clone().oneshot(req).await.expect("response")).await);
        }
        assert_eq!(answers[0].status, StatusCode::CONFLICT);
        assert_eq!(answers[0].body["error"]["code"], "DUPLICATE_USERNAME");
        assert_eq!(answers[0], answers[1]);
    })
    .await
    .expect("test timed out");
}

#[cfg(feature = "sign-up")]
#[tokio::test]
async fn sign_up_rejects_a_password_equal_to_the_email() {
    timeout(TEST_TIMEOUT, async {
        let app = test_app(RouterConfig::default());
        let req = post(
            "/api/v1/auth/sign-up",
            peer("198.51.100.9"),
            None,
            json!({
                "email": "alice@ironflow.dev",
                "username": "alice",
                "password": "alice@ironflow.dev"
            }),
        );
        let resp = Answer::read(app.oneshot(req).await.expect("response")).await;
        assert_eq!(resp.status, StatusCode::BAD_REQUEST);
        assert_eq!(resp.body["error"]["code"], "WEAK_PASSWORD");
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn change_password_refuses_a_weak_new_password_and_keeps_the_old_one() {
    timeout(TEST_TIMEOUT, async {
        let harness = Harness::new(RouterConfig::default());
        let (user, auth) = harness.user("alice", PASSWORD, false).await;

        for weak in [
            user.email.as_str(),
            "Password2024!",
            "alice-is-the-best",
            "short",
        ] {
            let req = authed(
                "PATCH",
                "/api/v1/auth/password",
                &auth,
                json!({"old_password": PASSWORD, "new_password": weak}),
            );
            let resp =
                Answer::read(harness.app.clone().oneshot(req).await.expect("response")).await;
            assert_eq!(resp.status, StatusCode::BAD_REQUEST, "{weak}");
            assert_eq!(resp.body["error"]["code"], "WEAK_PASSWORD", "{weak}");
        }

        assert_eq!(
            harness.sign_in(&user.email, PASSWORD).await,
            StatusCode::NO_CONTENT,
            "the old password must still work"
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn change_password_accepts_a_strong_new_password() {
    timeout(TEST_TIMEOUT, async {
        let harness = Harness::new(RouterConfig::default());
        let (user, auth) = harness.user("alice", PASSWORD, false).await;
        let req = authed(
            "PATCH",
            "/api/v1/auth/password",
            &auth,
            json!({"old_password": PASSWORD, "new_password": OTHER_PASSWORD}),
        );
        let resp = harness.app.clone().oneshot(req).await.expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        assert_eq!(
            harness.sign_in(&user.email, OTHER_PASSWORD).await,
            StatusCode::NO_CONTENT
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn admin_cannot_create_a_user_with_a_weak_password() {
    timeout(TEST_TIMEOUT, async {
        let harness = Harness::new(RouterConfig::default());
        let (_, auth) = harness.user("admin", PASSWORD, true).await;

        let weak = authed(
            "POST",
            "/api/v1/users",
            &auth,
            json!({
                "email": "dave@ironflow.dev",
                "username": "dave",
                "password": "dave@ironflow.dev",
                "is_admin": false
            }),
        );
        let resp = Answer::read(harness.app.clone().oneshot(weak).await.expect("response")).await;
        assert_eq!(resp.status, StatusCode::BAD_REQUEST);
        assert_eq!(resp.body["error"]["code"], "WEAK_PASSWORD");
        assert_eq!(
            harness
                .sign_in("dave@ironflow.dev", "dave@ironflow.dev")
                .await,
            StatusCode::UNAUTHORIZED,
            "the refused account must not exist"
        );

        let strong = authed(
            "POST",
            "/api/v1/users",
            &auth,
            json!({
                "email": "dave@ironflow.dev",
                "username": "dave",
                "password": OTHER_PASSWORD,
                "is_admin": false
            }),
        );
        let resp = harness.app.clone().oneshot(strong).await.expect("response");
        assert_eq!(resp.status(), StatusCode::CREATED);
        assert_eq!(
            harness.sign_in("dave@ironflow.dev", OTHER_PASSWORD).await,
            StatusCode::NO_CONTENT
        );
    })
    .await
    .expect("test timed out");
}
