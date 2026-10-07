//! Log hygiene of the proxy: no token, credential or secret ever reaches the
//! logs.
//!
//! These tests share one process-global JSON subscriber, installed once by
//! [`captured`]. A global subscriber is the only capture that parallel tests
//! cannot disturb (a thread-local subscriber races with the callsite interest
//! cache of sibling tests). Every test asserts on values only it produces.

use std::io::{self, Write};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::serve;
use reqwest::{Client, StatusCode};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::spawn;
use tokio::time::timeout;
use tracing::Level;
use tracing::subscriber::set_global_default;
use tracing_subscriber::fmt as tracing_fmt;
use tracing_subscriber::fmt::MakeWriter;
use url::Url;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use ironflow_auth_proxy::{AuthProxyConfig, AuthProxyState, router};
use ironflow_core::auth_proxy::{AuthProxyRegistry, CredentialKind, ProxyCredential, TokenRequest};

const ADMIN_KEY: &str = "0123456789abcdef0123456789abcdef";
const OAUTH: &str = "sk-ant-oat01-test";
const SECRET: &str = "ghp_test";

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// Start the proxy relaying the Anthropic API and every `/r/` host to
/// `upstream`, and return its base URL.
async fn start_proxy(upstream: &str) -> String {
    let upstream = Url::parse(upstream).unwrap();
    let config = AuthProxyConfig::new(ADMIN_KEY)
        .with_upstream(upstream.clone())
        .with_secret_upstream(upstream);
    let state = AuthProxyState::with_registry(config, AuthProxyRegistry::default()).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(state);
    spawn(async move {
        serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

/// Log sink shared between the subscriber and the tests.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).to_string()
    }
}

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Captured {
    type Writer = Captured;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// The capture of the global JSON subscriber, installed on first use.
fn captured() -> &'static Captured {
    static CAPTURED: OnceLock<Captured> = OnceLock::new();
    CAPTURED.get_or_init(|| {
        let captured = Captured::default();
        let subscriber = tracing_fmt()
            .json()
            .with_writer(captured.clone())
            .with_max_level(Level::INFO)
            .with_ansi(false)
            .finish();
        set_global_default(subscriber).expect("global subscriber installed once");
        captured
    })
}

#[tokio::test]
async fn logs_never_contain_token_or_credential() {
    let captured = captured();

    timeout(Duration::from_secs(10), async {
        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "msg_1"})))
            .mount(&upstream)
            .await;
        let base = start_proxy(&upstream.uri()).await;
        let http = Client::new();

        let request = TokenRequest {
            run_id: "run-1".to_string(),
            step: "review".to_string(),
            expires_at: now() + 600,
            credential: ProxyCredential::new(CredentialKind::OauthToken, OAUTH.to_string()).into(),
        };
        let issued = http
            .post(format!("{base}/admin/v1/tokens"))
            .bearer_auth(ADMIN_KEY)
            .json(&request)
            .send()
            .await
            .unwrap();
        assert_eq!(issued.status(), StatusCode::CREATED);
        let body: Value = issued.json().await.unwrap();
        let id = body["id"].as_str().unwrap().to_string();
        let token = body["token"].as_str().unwrap().to_string();

        let resp = http
            .post(format!("{base}/v1/messages"))
            .bearer_auth(&token)
            .json(&json!({"model": "claude-sonnet-4-5", "messages": []}))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let rejected = http
            .post(format!("{base}/v1/messages"))
            .bearer_auth("ifap_unknown_token_value")
            .json(&json!({"model": "claude-sonnet-4-5", "messages": []}))
            .send()
            .await
            .unwrap();
        assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED);

        let logs = captured.text();
        assert!(logs.contains("token issued"), "{logs}");
        assert!(logs.contains("relayed"), "{logs}");
        assert!(logs.contains(&id[..12]), "{logs}");
        assert!(!logs.contains(&token), "{logs}");
        assert!(!logs.contains(OAUTH), "{logs}");
        assert!(!logs.contains("ifap_unknown_token_value"), "{logs}");
        assert!(!logs.contains(ADMIN_KEY), "{logs}");
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn secret_relay_logs_result_and_never_the_secret() {
    let captured = captured();

    timeout(Duration::from_secs(10), async {
        let upstream = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/user"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&upstream)
            .await;
        let base = start_proxy(&upstream.uri()).await;
        let http = Client::new();

        let issued = http
            .post(format!("{base}/admin/v1/tokens"))
            .bearer_auth(ADMIN_KEY)
            .json(&json!({
                "run_id": "run-2",
                "step": "publish",
                "expires_at": now() + 600,
                "credential": {
                    "name": "GITHUB_TOKEN",
                    "value": SECRET,
                    "injection": "bearer",
                    "hosts": ["api.github.com"],
                }
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(issued.status(), StatusCode::CREATED);
        let body: Value = issued.json().await.unwrap();
        let id = body["id"].as_str().unwrap().to_string();
        let token = body["token"].as_str().unwrap().to_string();

        let relayed = http
            .get(format!("{base}/r/api.github.com/user?per_page=5"))
            .bearer_auth(&token)
            .header("x-trace", "header-value-never-logged")
            .send()
            .await
            .unwrap();
        assert_eq!(relayed.status(), StatusCode::OK);
        let refused = http
            .get(format!("{base}/r/gitlab.com/user"))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap();
        assert_eq!(refused.status(), StatusCode::FORBIDDEN);

        let logs = captured.text();
        assert!(logs.contains("secret relay"), "{logs}");
        assert!(logs.contains(r#""result":"relayed""#), "{logs}");
        assert!(logs.contains(r#""result":"forbidden_host""#), "{logs}");
        assert!(logs.contains(r#""secret":"GITHUB_TOKEN""#), "{logs}");
        assert!(logs.contains(r#""host":"api.github.com""#), "{logs}");
        assert!(logs.contains(r#""host":"gitlab.com""#), "{logs}");
        assert!(logs.contains(r#""upstream_status":200"#), "{logs}");
        assert!(logs.contains(r#""run_id":"run-2""#), "{logs}");
        assert!(logs.contains(&id[..12]), "{logs}");
        assert!(!logs.contains(&token), "{logs}");
        assert!(!logs.contains(SECRET), "{logs}");
        assert!(!logs.contains("per_page"), "{logs}");
        assert!(!logs.contains("header-value-never-logged"), "{logs}");
    })
    .await
    .expect("test timed out");
}
