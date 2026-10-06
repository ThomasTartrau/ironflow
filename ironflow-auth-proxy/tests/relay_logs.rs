//! Log hygiene of the proxy: no token or credential ever reaches the logs.
//!
//! This test lives alone in its binary on purpose. A process-global subscriber
//! is the only capture that parallel tests cannot disturb (a thread-local
//! subscriber races with the callsite interest cache of sibling tests).
//! `set_global_default` succeeds once per process: never add another test here.

use std::io::{self, Write};
use std::sync::{Arc, Mutex};
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

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// Start the proxy relaying to `upstream` and return its base URL.
async fn start_proxy(upstream: &str) -> String {
    let config = AuthProxyConfig::new(ADMIN_KEY).with_upstream(Url::parse(upstream).unwrap());
    let state = AuthProxyState::with_registry(config, AuthProxyRegistry::default()).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(state);
    spawn(async move {
        serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

/// Log sink shared between the subscriber and the test.
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

#[tokio::test]
async fn logs_never_contain_token_or_credential() {
    let captured = Captured::default();
    let subscriber = tracing_fmt()
        .with_writer(captured.clone())
        .with_max_level(Level::INFO)
        .with_ansi(false)
        .finish();
    set_global_default(subscriber).expect("global subscriber installed once");

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
            credential: ProxyCredential::new(CredentialKind::OauthToken, OAUTH.to_string()),
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
