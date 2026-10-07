//! The `/r/<host>/` relay of proxied secrets: the real proxy router on a real
//! TCP port, relaying to a fake third-party API (wiremock) through the real
//! reqwest client. `with_secret_upstream` sends every allowlisted host to the
//! fake API, so the requested host only drives the allowlist.

use std::future::Future;
use std::net::SocketAddr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::serve;
use reqwest::redirect::Policy;
use reqwest::{Client, Method, Response, StatusCode};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::spawn;
use tokio::time::timeout;
use url::Url;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use ironflow_auth_proxy::{AuthProxyConfig, AuthProxyState, router};
use ironflow_core::auth_proxy::{
    AuthProxyRegistry, CredentialKind, HostPattern, ProxyCredential, SecretCredential,
    SecretInjection, TokenRequest,
};

const ADMIN_KEY: &str = "0123456789abcdef0123456789abcdef";
const OAUTH: &str = "sk-ant-oat01-test";
const SECRET: &str = "ghp_test";
/// `Basic base64("oauth2:glpat-test")`.
const GITLAB_BASIC: &str = "Basic b2F1dGgyOmdscGF0LXRlc3Q=";

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
    /// Start the proxy relaying both the Anthropic API and every `/r/` host
    /// to `upstream`.
    async fn start(upstream: &str) -> Self {
        let upstream = Url::parse(upstream).unwrap();
        let config = AuthProxyConfig::new(ADMIN_KEY)
            .with_upstream(upstream.clone())
            .with_secret_upstream(upstream);
        let state = AuthProxyState::with_registry(config, AuthProxyRegistry::default()).unwrap();
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

    /// Ask the admin API for a secret token of `value`, without checking
    /// the answer.
    async fn issue_secret(&self, value: &str, injection: Value, hosts: Value) -> Response {
        let request = json!({
            "run_id": "run-1",
            "step": "review",
            "expires_at": now() + 600,
            "credential": {
                "name": "GITHUB_TOKEN",
                "value": value,
                "injection": injection,
                "hosts": hosts,
            }
        });
        self.http
            .post(format!("{}/admin/v1/tokens", self.base))
            .bearer_auth(ADMIN_KEY)
            .json(&request)
            .send()
            .await
            .unwrap()
    }

    /// Issue a secret token of `value` and return its id and token.
    async fn secret_token(&self, value: &str, injection: Value, hosts: Value) -> (String, String) {
        let resp = self.issue_secret(value, injection, hosts).await;
        assert_eq!(resp.status(), StatusCode::CREATED);
        let body: Value = resp.json().await.unwrap();
        (
            body["id"].as_str().unwrap().to_string(),
            body["token"].as_str().unwrap().to_string(),
        )
    }

    /// Issue a bearer token of [`SECRET`] for `api.github.com`.
    async fn github_token(&self) -> (String, String) {
        self.secret_token(SECRET, json!("bearer"), json!(["api.github.com"]))
            .await
    }

    /// `GET /r/<target>` with `token` as a bearer token.
    async fn get(&self, target: &str, token: &str) -> Response {
        self.http
            .get(format!("{}/r/{target}", self.base))
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
    }

    /// Send `target` as is over a raw connection, so that no client
    /// normalizes it, and return the raw answer.
    async fn raw_get(&self, target: &str, token: &str) -> String {
        let mut stream = TcpStream::connect(self.addr).await.unwrap();
        let request = format!(
            "GET {target} HTTP/1.1\r\nHost: localhost\r\n\
             Authorization: Bearer {token}\r\nConnection: close\r\n\r\n"
        );
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        String::from_utf8_lossy(&raw).to_string()
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
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    server
}

/// An upstream answering 200 to `GET <route>` when `name: value` is sent,
/// exactly once.
async fn upstream_expecting(route: &str, name: &'static str, value: &str) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(route))
        .and(header(name, value))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .expect(1)
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn bearer_secret_is_relayed_with_real_secret() {
    within_timeout(async {
        let upstream = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/user"))
            .and(query_param("per_page", "5"))
            .and(header("authorization", format!("Bearer {SECRET}").as_str()))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"login": "bot"})))
            .expect(1)
            .mount(&upstream)
            .await;
        let proxy = Proxy::start(&upstream.uri()).await;
        let (_, token) = proxy.github_token().await;

        let resp = proxy.get("api.github.com/user?per_page=5", &token).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body: Value = resp.json().await.unwrap();
        assert_eq!(body["login"], "bot");
    })
    .await;
}

#[tokio::test]
async fn private_token_secret_is_relayed() {
    within_timeout(async {
        let upstream = upstream_expecting("/api/v4/user", "private-token", "glpat-test").await;
        let proxy = Proxy::start(&upstream.uri()).await;
        let (_, token) = proxy
            .secret_token("glpat-test", json!("private_token"), json!(["gitlab.com"]))
            .await;

        // The pod sends its token where the real one would go.
        let resp = proxy
            .http
            .get(format!("{}/r/gitlab.com/api/v4/user", proxy.base))
            .header("private-token", &token)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    })
    .await;
}

#[tokio::test]
async fn x_api_key_secret_is_relayed() {
    within_timeout(async {
        let upstream = upstream_expecting("/v1/items", "x-api-key", "key-test").await;
        let proxy = Proxy::start(&upstream.uri()).await;
        let (_, token) = proxy
            .secret_token("key-test", json!("x_api_key"), json!(["api.example.com"]))
            .await;

        let resp = proxy
            .http
            .get(format!("{}/r/api.example.com/v1/items", proxy.base))
            .header("x-api-key", &token)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    })
    .await;
}

#[tokio::test]
async fn custom_header_secret_is_relayed() {
    within_timeout(async {
        let upstream = upstream_expecting("/v1/secret", "x-vault-token", "hvs-test").await;
        let proxy = Proxy::start(&upstream.uri()).await;
        let injection = json!({"header": "X-Vault-Token"});
        let (_, token) = proxy
            .secret_token("hvs-test", injection, json!(["vault.example.com"]))
            .await;

        // A forged value of the injected header is replaced, not forwarded.
        let resp = proxy
            .http
            .get(format!("{}/r/vault.example.com/v1/secret", proxy.base))
            .bearer_auth(&token)
            .header("x-vault-token", "forged")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let received: Vec<Request> = upstream.received_requests().await.unwrap();
        let values: Vec<&str> = received[0]
            .headers
            .get_all("x-vault-token")
            .iter()
            .map(|value| value.to_str().unwrap())
            .collect();
        assert_eq!(values, ["hvs-test"]);
    })
    .await;
}

#[tokio::test]
async fn basic_secret_is_relayed_as_password() {
    within_timeout(async {
        let upstream = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/group/repo.git/info/refs"))
            .and(query_param("service", "git-upload-pack"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&upstream)
            .await;
        let proxy = Proxy::start(&upstream.uri()).await;
        let injection = json!({"basic": {"username": "oauth2"}});
        let (_, token) = proxy
            .secret_token("glpat-test", injection, json!(["gitlab.com"]))
            .await;

        // git sends its credentials as Basic: the token is the password.
        let resp = proxy
            .http
            .get(format!(
                "{}/r/gitlab.com/group/repo.git/info/refs?service=git-upload-pack",
                proxy.base
            ))
            .basic_auth("x-access-token", Some(&token))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let received = upstream.received_requests().await.unwrap();
        assert_eq!(received[0].headers["authorization"], GITLAB_BASIC);
    })
    .await;
}

#[tokio::test]
async fn opaque_token_headers_are_stripped() {
    within_timeout(async {
        let upstream = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&upstream)
            .await;
        let proxy = Proxy::start(&upstream.uri()).await;
        let (_, token) = proxy.github_token().await;

        let resp = proxy
            .http
            .get(format!("{}/r/api.github.com/user", proxy.base))
            .bearer_auth(&token)
            .header("x-api-key", &token)
            .header("private-token", &token)
            .header("accept", "application/vnd.github+json")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let received = upstream.received_requests().await.unwrap();
        let headers = &received[0].headers;
        assert_eq!(headers["accept"], "application/vnd.github+json");
        for (name, value) in headers {
            let value = value.to_str().unwrap_or_default();
            assert!(!value.contains("ifap_"), "opaque token forwarded in {name}");
        }
    })
    .await;
}

#[tokio::test]
async fn host_outside_allowlist_returns_403() {
    within_timeout(async {
        let upstream = silent_upstream().await;
        let proxy = Proxy::start(&upstream.uri()).await;
        let (_, token) = proxy.github_token().await;

        let resp = proxy.get("gitlab.com/api/v4/user", &token).await;
        assert_auth_error(resp, StatusCode::FORBIDDEN, "permission_error").await;
    })
    .await;
}

#[tokio::test]
async fn lookalike_host_returns_403() {
    within_timeout(async {
        let upstream = silent_upstream().await;
        let proxy = Proxy::start(&upstream.uri()).await;
        let (_, token) = proxy.github_token().await;

        for target in [
            "api.github.com.evil.example/user",
            "evilapi.github.com/user",
            "x.api.github.com/user",
        ] {
            let resp = proxy.get(target, &token).await;
            assert_auth_error(resp, StatusCode::FORBIDDEN, "permission_error").await;
        }
    })
    .await;
}

#[tokio::test]
async fn wildcard_host_is_relayed() {
    within_timeout(async {
        let upstream = upstream_expecting("/v1/items", "authorization", "Bearer wild-test").await;
        let proxy = Proxy::start(&upstream.uri()).await;
        let (_, token) = proxy
            .secret_token("wild-test", json!("bearer"), json!(["*.example.org"]))
            .await;

        let resp = proxy.get("API.Example.org/v1/items", &token).await;
        assert_eq!(resp.status(), StatusCode::OK);
    })
    .await;
}

#[tokio::test]
async fn wildcard_does_not_match_apex() {
    within_timeout(async {
        let upstream = silent_upstream().await;
        let proxy = Proxy::start(&upstream.uri()).await;
        let (_, token) = proxy
            .secret_token("wild-test", json!("bearer"), json!(["*.example.org"]))
            .await;

        let resp = proxy.get("example.org/v1/items", &token).await;
        assert_auth_error(resp, StatusCode::FORBIDDEN, "permission_error").await;
    })
    .await;
}

#[tokio::test]
async fn invalid_host_returns_403() {
    within_timeout(async {
        let upstream = silent_upstream().await;
        let proxy = Proxy::start(&upstream.uri()).await;
        let (_, token) = proxy.github_token().await;

        for target in ["127.0.0.1/", "api.github.com:8443/"] {
            let resp = proxy.get(target, &token).await;
            assert_auth_error(resp, StatusCode::FORBIDDEN, "permission_error").await;
        }
    })
    .await;
}

#[tokio::test]
async fn path_traversal_returns_403() {
    within_timeout(async {
        let upstream = silent_upstream().await;
        let proxy = Proxy::start(&upstream.uri()).await;
        let (_, token) = proxy.github_token().await;

        for target in [
            "/r/api.github.com/a/../b",
            "/r/api.github.com//x",
            "/r/api.github.com/a/%2e%2e/b",
        ] {
            let answer = proxy.raw_get(target, &token).await;
            assert!(answer.starts_with("HTTP/1.1 403"), "{target}: {answer}");
            assert!(answer.contains("permission_error"), "{target}: {answer}");
        }
    })
    .await;
}

#[tokio::test]
async fn unsupported_method_returns_405() {
    within_timeout(async {
        let upstream = silent_upstream().await;
        let proxy = Proxy::start(&upstream.uri()).await;
        let (_, token) = proxy.github_token().await;

        let resp = proxy
            .http
            .request(
                Method::OPTIONS,
                format!("{}/r/api.github.com/user", proxy.base),
            )
            .bearer_auth(&token)
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
async fn post_body_is_relayed() {
    within_timeout(async {
        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/repos/o/r/issues"))
            .respond_with(ResponseTemplate::new(201))
            .expect(1)
            .mount(&upstream)
            .await;
        let proxy = Proxy::start(&upstream.uri()).await;
        let (_, token) = proxy.github_token().await;

        let resp = proxy
            .http
            .post(format!("{}/r/api.github.com/repos/o/r/issues", proxy.base))
            .bearer_auth(&token)
            .json(&json!({"title": "bug"}))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        let received = upstream.received_requests().await.unwrap();
        let sent: Value = received[0].body_json().unwrap();
        assert_eq!(sent["title"], "bug");
    })
    .await;
}

#[tokio::test]
async fn redirect_is_returned_not_followed() {
    within_timeout(async {
        let upstream = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(302).insert_header("location", "https://evil.example.net/"),
            )
            .expect(1)
            .mount(&upstream)
            .await;
        let proxy = Proxy::start(&upstream.uri()).await;
        let (_, token) = proxy.github_token().await;

        // The test client must not follow it either.
        let http = Client::builder().redirect(Policy::none()).build().unwrap();
        let resp = http
            .get(format!("{}/r/api.github.com/user", proxy.base))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FOUND);
        assert_eq!(resp.headers()["location"], "https://evil.example.net/");
        assert_eq!(upstream.received_requests().await.unwrap().len(), 1);
    })
    .await;
}

#[tokio::test]
async fn claude_token_on_relay_returns_403() {
    within_timeout(async {
        let upstream = silent_upstream().await;
        let proxy = Proxy::start(&upstream.uri()).await;
        let request = TokenRequest {
            run_id: "run-1".to_string(),
            step: "review".to_string(),
            expires_at: now() + 600,
            credential: ProxyCredential::new(CredentialKind::OauthToken, OAUTH.to_string()).into(),
        };
        let issued = proxy.state.registry().issue(request, now()).await.unwrap();

        let resp = proxy.get("api.github.com/user", &issued.token).await;
        assert_auth_error(resp, StatusCode::FORBIDDEN, "permission_error").await;
    })
    .await;
}

#[tokio::test]
async fn secret_token_on_anthropic_route_returns_403() {
    within_timeout(async {
        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&upstream)
            .await;
        let proxy = Proxy::start(&upstream.uri()).await;
        let (_, token) = proxy.github_token().await;

        let resp = proxy
            .http
            .post(format!("{}/v1/messages", proxy.base))
            .bearer_auth(&token)
            .json(&json!({"model": "claude-sonnet-4-5", "messages": []}))
            .send()
            .await
            .unwrap();
        assert_auth_error(resp, StatusCode::FORBIDDEN, "permission_error").await;
    })
    .await;
}

#[tokio::test]
async fn unknown_token_returns_401() {
    within_timeout(async {
        let upstream = silent_upstream().await;
        let proxy = Proxy::start(&upstream.uri()).await;

        let resp = proxy.get("api.github.com/user", "ifap_unknown").await;
        assert_auth_error(resp, StatusCode::UNAUTHORIZED, "authentication_error").await;
        let missing = proxy
            .http
            .get(format!("{}/r/api.github.com/user", proxy.base))
            .send()
            .await
            .unwrap();
        assert_auth_error(missing, StatusCode::UNAUTHORIZED, "authentication_error").await;
    })
    .await;
}

#[tokio::test]
async fn expired_secret_token_returns_401() {
    within_timeout(async {
        let upstream = silent_upstream().await;
        let proxy = Proxy::start(&upstream.uri()).await;
        // Issued in the past, expired ten seconds ago.
        let issued_at = now() - 100;
        let request = TokenRequest {
            run_id: "run-1".to_string(),
            step: "review".to_string(),
            expires_at: issued_at + 90,
            credential: SecretCredential::new(
                "GITHUB_TOKEN".to_string(),
                SECRET.to_string(),
                SecretInjection::Bearer,
                vec![HostPattern::parse("api.github.com").unwrap()],
            )
            .into(),
        };
        let issued = proxy
            .state
            .registry()
            .issue(request, issued_at)
            .await
            .unwrap();

        let resp = proxy.get("api.github.com/user", &issued.token).await;
        assert_auth_error(resp, StatusCode::UNAUTHORIZED, "authentication_error").await;
    })
    .await;
}

#[tokio::test]
async fn revoked_secret_token_returns_401() {
    within_timeout(async {
        let upstream = silent_upstream().await;
        let proxy = Proxy::start(&upstream.uri()).await;
        let (id, token) = proxy.github_token().await;

        let resp = proxy
            .http
            .delete(format!("{}/admin/v1/tokens/{id}", proxy.base))
            .bearer_auth(ADMIN_KEY)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        let resp = proxy.get("api.github.com/user", &token).await;
        assert_auth_error(resp, StatusCode::UNAUTHORIZED, "authentication_error").await;
    })
    .await;
}

#[tokio::test]
async fn revoke_run_revokes_secret_grants() {
    within_timeout(async {
        let upstream = silent_upstream().await;
        let proxy = Proxy::start(&upstream.uri()).await;
        let (_, github) = proxy.github_token().await;
        let (_, gitlab) = proxy
            .secret_token("glpat-test", json!("private_token"), json!(["gitlab.com"]))
            .await;

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

        let targets = [("api.github.com/user", github), ("gitlab.com/x", gitlab)];
        for (target, token) in targets {
            let resp = proxy.get(target, &token).await;
            assert_auth_error(resp, StatusCode::UNAUTHORIZED, "authentication_error").await;
        }
    })
    .await;
}

#[tokio::test]
async fn issue_secret_with_empty_hosts_returns_400() {
    within_timeout(async {
        let upstream = silent_upstream().await;
        let proxy = Proxy::start(&upstream.uri()).await;

        let hosts = json!([]);
        let resp = proxy.issue_secret(SECRET, json!("bearer"), hosts).await;
        assert_auth_error(resp, StatusCode::BAD_REQUEST, "invalid_request_error").await;
        assert!(proxy.state.registry().is_empty().await.unwrap());
    })
    .await;
}

#[tokio::test]
async fn issue_secret_with_regex_like_host_returns_400() {
    within_timeout(async {
        let upstream = silent_upstream().await;
        let proxy = Proxy::start(&upstream.uri()).await;

        for host in [
            ".*github.com",
            "https://api.github.com",
            "api.github.com:443",
        ] {
            let resp = proxy
                .issue_secret(SECRET, json!("bearer"), json!([host]))
                .await;
            let status = resp.status();
            let text = resp.text().await.unwrap();
            assert_eq!(status, StatusCode::BAD_REQUEST, "{host}");
            assert!(text.contains("invalid_request_error"), "{host}: {text}");
            // The refusal never echoes the secret value.
            assert!(!text.contains(SECRET), "{host}: {text}");
        }
        assert!(proxy.state.registry().is_empty().await.unwrap());
    })
    .await;
}
