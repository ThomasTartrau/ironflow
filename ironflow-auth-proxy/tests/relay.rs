//! The real proxy router on a real TCP port, relaying to a fake Anthropic API
//! (wiremock) through the real reqwest client.

use std::future::Future;
use std::net::{SocketAddr, TcpListener as StdTcpListener};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::serve;
use reqwest::{Client, Response, StatusCode};
use serde_json::{Value, from_slice, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::spawn;
use tokio::time::timeout;
use url::Url;
use wiremock::matchers::{header, header_exists, method, path, query_param};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use ironflow_auth_proxy::{AuthProxyConfig, AuthProxyState, router};
use ironflow_core::auth_proxy::{AuthProxyRegistry, CredentialKind, ProxyCredential, TokenRequest};

const ADMIN_KEY: &str = "0123456789abcdef0123456789abcdef";
const OAUTH: &str = "sk-ant-oat01-test";
const API_KEY: &str = "sk-ant-api03-test";

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

async fn within_timeout(test: impl Future<Output = ()>) {
    timeout(Duration::from_secs(10), test)
        .await
        .expect("test timed out");
}

struct Proxy {
    base: String,
    addr: SocketAddr,
    state: AuthProxyState,
    http: Client,
}

impl Proxy {
    /// Start the proxy relaying to `upstream`, with its own in-memory registry.
    async fn start(upstream: &str) -> Self {
        Self::start_with_registry(upstream, AuthProxyRegistry::default()).await
    }

    /// Start a proxy replica relaying to `upstream` over `registry`.
    async fn start_with_registry(upstream: &str, registry: AuthProxyRegistry) -> Self {
        let config = AuthProxyConfig::new(ADMIN_KEY).with_upstream(Url::parse(upstream).unwrap());
        let state = AuthProxyState::with_registry(config, registry).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = router(state.clone());
        spawn(async move {
            serve(listener, app).await.unwrap();
        });
        Self {
            base: format!("http://{addr}"),
            addr,
            state,
            http: Client::new(),
        }
    }

    /// Issue a token through the admin API.
    async fn issue(&self, kind: CredentialKind, value: &str) -> (String, String) {
        let request = TokenRequest {
            run_id: "run-1".to_string(),
            step: "review".to_string(),
            expires_at: now() + 600,
            credential: ProxyCredential::new(kind, value.to_string()).into(),
        };
        let resp = self
            .http
            .post(format!("{}/admin/v1/tokens", self.base))
            .bearer_auth(ADMIN_KEY)
            .json(&request)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let body: Value = resp.json().await.unwrap();
        (
            body["id"].as_str().unwrap().to_string(),
            body["token"].as_str().unwrap().to_string(),
        )
    }

    async fn post_messages(&self, token: &str) -> Response {
        self.http
            .post(format!("{}/v1/messages", self.base))
            .bearer_auth(token)
            .json(&json!({"model": "claude-sonnet-4-5", "messages": []}))
            .send()
            .await
            .unwrap()
    }
}

async fn assert_auth_error(resp: Response, status: StatusCode, kind: &str) {
    assert_eq!(resp.status(), status);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["type"], "error");
    assert_eq!(body["error"]["type"], kind);
}

/// An upstream that must never be reached.
async fn silent_upstream() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn unknown_token_returns_401() {
    within_timeout(async {
        let upstream = silent_upstream().await;
        let proxy = Proxy::start(&upstream.uri()).await;
        let resp = proxy.post_messages("invalide").await;
        assert_auth_error(resp, StatusCode::UNAUTHORIZED, "authentication_error").await;
    })
    .await;
}

#[tokio::test]
async fn missing_token_returns_401() {
    within_timeout(async {
        let upstream = silent_upstream().await;
        let proxy = Proxy::start(&upstream.uri()).await;
        let resp = proxy
            .http
            .post(format!("{}/v1/messages", proxy.base))
            .body("{}")
            .send()
            .await
            .unwrap();
        assert_auth_error(resp, StatusCode::UNAUTHORIZED, "authentication_error").await;
    })
    .await;
}

#[tokio::test]
async fn expired_token_returns_401() {
    within_timeout(async {
        let upstream = silent_upstream().await;
        let proxy = Proxy::start(&upstream.uri()).await;
        // Issued in the past, expired ten seconds ago.
        let issued_at = now() - 100;
        let request = TokenRequest {
            run_id: "run-1".to_string(),
            step: "review".to_string(),
            expires_at: issued_at + 90,
            credential: ProxyCredential::new(CredentialKind::OauthToken, OAUTH.to_string()).into(),
        };
        let issued = proxy
            .state
            .registry()
            .issue(request, issued_at)
            .await
            .unwrap();
        let resp = proxy.post_messages(&issued.token).await;
        assert_auth_error(resp, StatusCode::UNAUTHORIZED, "authentication_error").await;
        assert!(proxy.state.registry().is_empty().await.unwrap());
    })
    .await;
}

#[tokio::test]
async fn revoked_token_returns_401() {
    within_timeout(async {
        let upstream = silent_upstream().await;
        let proxy = Proxy::start(&upstream.uri()).await;
        let (id, token) = proxy.issue(CredentialKind::OauthToken, OAUTH).await;

        let resp = proxy
            .http
            .delete(format!("{}/admin/v1/tokens/{id}", proxy.base))
            .bearer_auth(ADMIN_KEY)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
        let again = proxy
            .http
            .delete(format!("{}/admin/v1/tokens/{id}", proxy.base))
            .bearer_auth(ADMIN_KEY)
            .send()
            .await
            .unwrap();
        assert_eq!(again.status(), StatusCode::NOT_FOUND);

        let resp = proxy.post_messages(&token).await;
        assert_auth_error(resp, StatusCode::UNAUTHORIZED, "authentication_error").await;
    })
    .await;
}

#[tokio::test]
async fn token_issued_by_one_replica_is_relayed_by_another() {
    within_timeout(async {
        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .and(header("authorization", format!("Bearer {OAUTH}").as_str()))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "msg_1"})))
            .expect(1)
            .mount(&upstream)
            .await;
        let registry = AuthProxyRegistry::default();
        let a = Proxy::start_with_registry(&upstream.uri(), registry.clone()).await;
        let b = Proxy::start_with_registry(&upstream.uri(), registry).await;
        assert_ne!(a.addr, b.addr);

        let (id, token) = a.issue(CredentialKind::OauthToken, OAUTH).await;
        let resp = b.post_messages(&token).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body: Value = resp.json().await.unwrap();
        assert_eq!(body["id"], "msg_1");

        let revoke = b
            .http
            .delete(format!("{}/admin/v1/tokens/{id}", b.base))
            .bearer_auth(ADMIN_KEY)
            .send()
            .await
            .unwrap();
        assert_eq!(revoke.status(), StatusCode::NO_CONTENT);

        let resp = a.post_messages(&token).await;
        assert_auth_error(resp, StatusCode::UNAUTHORIZED, "authentication_error").await;
    })
    .await;
}

#[tokio::test]
async fn revoke_run_drops_every_token_of_the_run() {
    within_timeout(async {
        let upstream = silent_upstream().await;
        let proxy = Proxy::start(&upstream.uri()).await;
        let (_, first) = proxy.issue(CredentialKind::OauthToken, OAUTH).await;
        let (_, second) = proxy.issue(CredentialKind::ApiKey, API_KEY).await;

        let resp = proxy
            .http
            .delete(format!("{}/admin/v1/runs/run-1/tokens", proxy.base))
            .bearer_auth(ADMIN_KEY)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body: Value = resp.json().await.unwrap();
        assert_eq!(body["revoked"], 2);

        for token in [first, second] {
            let resp = proxy.post_messages(&token).await;
            assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        }
    })
    .await;
}

#[tokio::test]
async fn valid_oauth_token_relays_with_real_credential() {
    within_timeout(async {
        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .and(query_param("beta", "true"))
            .and(header("authorization", format!("Bearer {OAUTH}").as_str()))
            .and(header("anthropic-version", "2023-06-01"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "msg_1"})))
            .expect(1)
            .mount(&upstream)
            .await;
        let proxy = Proxy::start(&upstream.uri()).await;
        let (_, token) = proxy.issue(CredentialKind::OauthToken, OAUTH).await;

        let resp = proxy
            .http
            .post(format!("{}/v1/messages?beta=true", proxy.base))
            .bearer_auth(&token)
            .header("anthropic-version", "2023-06-01")
            .header("anthropic-beta", "claude-code-20250219")
            .json(&json!({"model": "claude-sonnet-4-5", "messages": []}))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body: Value = resp.json().await.unwrap();
        assert_eq!(body["id"], "msg_1");

        let received: Vec<Request> = upstream.received_requests().await.unwrap();
        assert_eq!(received.len(), 1);
        let headers = &received[0].headers;
        let beta = headers.get("anthropic-beta").unwrap().to_str().unwrap();
        assert!(beta.contains("oauth-2025-04-20"), "{beta}");
        assert!(beta.contains("claude-code-20250219"), "{beta}");
        assert!(headers.get("x-api-key").is_none());
        for (_, value) in headers {
            let value = value.to_str().unwrap_or_default();
            assert!(!value.contains(&token), "opaque token forwarded upstream");
        }
        let sent: Value = from_slice(&received[0].body).unwrap();
        assert_eq!(sent["model"], "claude-sonnet-4-5");
    })
    .await;
}

#[tokio::test]
async fn valid_api_key_token_relays_with_x_api_key() {
    within_timeout(async {
        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages/count_tokens"))
            .and(header("x-api-key", API_KEY))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"input_tokens": 3})))
            .expect(1)
            .mount(&upstream)
            .await;
        let proxy = Proxy::start(&upstream.uri()).await;
        let (_, token) = proxy.issue(CredentialKind::ApiKey, API_KEY).await;

        let resp = proxy
            .http
            .post(format!("{}/v1/messages/count_tokens", proxy.base))
            .header("x-api-key", &token)
            .body("{}")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let received = upstream.received_requests().await.unwrap();
        assert!(received[0].headers.get("authorization").is_none());
    })
    .await;
}

#[tokio::test]
async fn sse_response_is_streamed_with_ratelimit_headers() {
    within_timeout(async {
        let events = concat!(
            "event: message_start\ndata: {\"type\":\"message_start\"}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\"}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        );
        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .and(header_exists("authorization"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("anthropic-ratelimit-unified-status", "allowed")
                    .insert_header("anthropic-ratelimit-unified-5h-utilization", "0.42")
                    .insert_header("request-id", "req_123")
                    .set_body_raw(events, "text/event-stream"),
            )
            .mount(&upstream)
            .await;
        let proxy = Proxy::start(&upstream.uri()).await;
        let (_, token) = proxy.issue(CredentialKind::OauthToken, OAUTH).await;

        let resp = proxy.post_messages(&token).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let headers = resp.headers();
        assert_eq!(headers["content-type"], "text/event-stream");
        assert_eq!(headers["anthropic-ratelimit-unified-status"], "allowed");
        assert_eq!(
            headers["anthropic-ratelimit-unified-5h-utilization"],
            "0.42"
        );
        assert_eq!(headers["request-id"], "req_123");
        assert_eq!(resp.text().await.unwrap(), events);
    })
    .await;
}

#[tokio::test]
async fn path_outside_api_returns_403() {
    within_timeout(async {
        let upstream = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&upstream)
            .await;
        let proxy = Proxy::start(&upstream.uri()).await;
        let (_, token) = proxy.issue(CredentialKind::OauthToken, OAUTH).await;

        for target in [
            "/api/oauth/profile",
            "/admin/v1/tokens",
            "/",
            "/v1/%2e%2e/x",
        ] {
            let resp = proxy
                .http
                .get(format!("{}{target}", proxy.base))
                .bearer_auth(&token)
                .send()
                .await
                .unwrap();
            if target == "/admin/v1/tokens" {
                // The admin route exists for POST only: never relayed.
                assert_ne!(resp.status(), StatusCode::OK, "{target}");
                continue;
            }
            assert_auth_error(resp, StatusCode::FORBIDDEN, "permission_error").await;
        }
    })
    .await;
}

#[tokio::test]
async fn absolute_form_request_to_other_host_returns_403() {
    within_timeout(async {
        let upstream = silent_upstream().await;
        let proxy = Proxy::start(&upstream.uri()).await;
        let (_, token) = proxy.issue(CredentialKind::OauthToken, OAUTH).await;

        let mut stream = TcpStream::connect(proxy.addr).await.unwrap();
        let request = format!(
            "GET http://example.com/v1/messages HTTP/1.1\r\nHost: example.com\r\n\
             Authorization: Bearer {token}\r\nConnection: close\r\n\r\n"
        );
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let answer = String::from_utf8_lossy(&raw);
        assert!(answer.starts_with("HTTP/1.1 403"), "{answer}");
        assert!(answer.contains("permission_error"), "{answer}");
    })
    .await;
}

#[tokio::test]
async fn put_method_returns_405() {
    within_timeout(async {
        let upstream = silent_upstream().await;
        let proxy = Proxy::start(&upstream.uri()).await;
        let (_, token) = proxy.issue(CredentialKind::OauthToken, OAUTH).await;

        let resp = proxy
            .http
            .put(format!("{}/v1/messages", proxy.base))
            .bearer_auth(&token)
            .body("{}")
            .send()
            .await
            .unwrap();
        assert_auth_error(
            resp,
            StatusCode::METHOD_NOT_ALLOWED,
            "invalid_request_error",
        )
        .await;
    })
    .await;
}

#[tokio::test]
async fn upstream_unreachable_returns_502() {
    within_timeout(async {
        // Bind then drop: nothing listens on the port any more.
        let port = {
            let listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap().port()
        };
        let proxy = Proxy::start(&format!("http://127.0.0.1:{port}")).await;
        let (_, token) = proxy.issue(CredentialKind::OauthToken, OAUTH).await;

        let resp = proxy.post_messages(&token).await;
        assert_auth_error(resp, StatusCode::BAD_GATEWAY, "api_error").await;
    })
    .await;
}

#[tokio::test]
async fn healthz_returns_ok() {
    within_timeout(async {
        let upstream = silent_upstream().await;
        let proxy = Proxy::start(&upstream.uri()).await;
        let resp = proxy
            .http
            .get(format!("{}/healthz", proxy.base))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.text().await.unwrap(), "ok");
    })
    .await;
}

#[tokio::test]
async fn admin_without_key_returns_401() {
    within_timeout(async {
        let upstream = silent_upstream().await;
        let proxy = Proxy::start(&upstream.uri()).await;
        let body = json!({
            "run_id": "run-1",
            "step": "review",
            "expires_at": now() + 600,
            "credential": {"kind": "oauth_token", "value": OAUTH}
        });

        let missing = proxy
            .http
            .post(format!("{}/admin/v1/tokens", proxy.base))
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_auth_error(missing, StatusCode::UNAUTHORIZED, "authentication_error").await;

        let wrong = proxy
            .http
            .post(format!("{}/admin/v1/tokens", proxy.base))
            .bearer_auth("ifap_not_the_admin_key_0123456789abcdef")
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_auth_error(wrong, StatusCode::UNAUTHORIZED, "authentication_error").await;

        let revoke = proxy
            .http
            .delete(format!("{}/admin/v1/runs/run-1/tokens", proxy.base))
            .send()
            .await
            .unwrap();
        assert_eq!(revoke.status(), StatusCode::UNAUTHORIZED);
        assert!(proxy.state.registry().is_empty().await.unwrap());
    })
    .await;
}

#[tokio::test]
async fn admin_issue_rejects_past_expiry_with_400() {
    within_timeout(async {
        let upstream = silent_upstream().await;
        let proxy = Proxy::start(&upstream.uri()).await;
        let past = json!({
            "run_id": "run-1",
            "step": "review",
            "expires_at": now() - 1,
            "credential": {"kind": "oauth_token", "value": OAUTH}
        });
        let resp = proxy
            .http
            .post(format!("{}/admin/v1/tokens", proxy.base))
            .bearer_auth(ADMIN_KEY)
            .json(&past)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body: Value = resp.json().await.unwrap();
        assert_eq!(body["error"]["type"], "invalid_request_error");
        assert_eq!(body["error"]["message"], "expires_at is in the past");

        let garbage = proxy
            .http
            .post(format!("{}/admin/v1/tokens", proxy.base))
            .bearer_auth(ADMIN_KEY)
            .body(format!("{{\"credential\": \"{OAUTH}\"}}"))
            .send()
            .await
            .unwrap();
        assert_eq!(garbage.status(), StatusCode::BAD_REQUEST);
        let text = garbage.text().await.unwrap();
        assert!(!text.contains(OAUTH), "{text}");
        assert!(proxy.state.registry().is_empty().await.unwrap());
    })
    .await;
}
