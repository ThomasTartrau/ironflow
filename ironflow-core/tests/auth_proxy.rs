//! The real [`AuthProxyClient`] against a fake admin API on a real TCP port
//! (wiremock).

use std::net::TcpListener;
use std::time::Duration;

use serde_json::json;
use tokio::time::timeout;
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use ironflow_core::auth_proxy::{
    AuthProxyClient, AuthProxyError, CredentialKind, ProxyCredential, TokenRequest,
};

const ADMIN_KEY: &str = "0123456789abcdef0123456789abcdef";
const CREDENTIAL: &str = "sk-ant-oat01-client-test";

fn request() -> TokenRequest {
    TokenRequest {
        run_id: "run-1".to_string(),
        step: "review".to_string(),
        expires_at: 1_700_000_600,
        credential: ProxyCredential::new(CredentialKind::OauthToken, CREDENTIAL.to_string()),
    }
}

#[tokio::test]
async fn auth_proxy_client_issue_sends_admin_bearer_and_grant() {
    timeout(Duration::from_secs(10), async {
        let server = MockServer::start().await;
        let issued = json!({"id": "abc123", "token": "ifap_xyz"});
        Mock::given(method("POST"))
            .and(path("/admin/v1/tokens"))
            .and(header(
                "authorization",
                format!("Bearer {ADMIN_KEY}").as_str(),
            ))
            .and(body_partial_json(json!({
                "run_id": "run-1",
                "step": "review",
                "expires_at": 1_700_000_600u64,
                "credential": { "kind": "oauth_token", "value": CREDENTIAL }
            })))
            .respond_with(ResponseTemplate::new(201).set_body_json(issued))
            .expect(1)
            .mount(&server)
            .await;

        let client = AuthProxyClient::new(&format!("{}/", server.uri()), ADMIN_KEY);
        let issued = client.issue(&request()).await.unwrap();
        assert_eq!(issued.id, "abc123");
        assert_eq!(issued.token, "ifap_xyz");
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn auth_proxy_client_issue_maps_401_to_admin_error() {
    timeout(Duration::from_secs(10), async {
        let server = MockServer::start().await;
        let long_body = "x".repeat(500);
        Mock::given(method("POST"))
            .and(path("/admin/v1/tokens"))
            .respond_with(ResponseTemplate::new(401).set_body_string(long_body))
            .mount(&server)
            .await;

        let client = AuthProxyClient::new(&server.uri(), "wrong-key");
        match client.issue(&request()).await {
            Err(AuthProxyError::Admin { status, message }) => {
                assert_eq!(status, 401);
                assert_eq!(message.chars().count(), 200);
            }
            other => panic!("expected an admin error, got {other:?}"),
        }
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn auth_proxy_client_issue_rejects_unexpected_success_status() {
    timeout(Duration::from_secs(10), async {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/admin/v1/tokens"))
            .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
            .mount(&server)
            .await;

        let client = AuthProxyClient::new(&server.uri(), ADMIN_KEY);
        let err = client.issue(&request()).await.unwrap_err();
        assert!(
            matches!(err, AuthProxyError::Admin { status: 200, .. }),
            "{err}"
        );
        assert!(!err.to_string().contains(CREDENTIAL));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn auth_proxy_client_revoke_accepts_404() {
    timeout(Duration::from_secs(10), async {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/admin/v1/tokens/gone"))
            .and(header(
                "authorization",
                format!("Bearer {ADMIN_KEY}").as_str(),
            ))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/admin/v1/tokens/live"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/admin/v1/tokens/broken"))
            .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
            .mount(&server)
            .await;

        let client = AuthProxyClient::new(&server.uri(), ADMIN_KEY);
        client.revoke("gone").await.unwrap();
        client.revoke("live").await.unwrap();
        match client.revoke("broken").await {
            Err(AuthProxyError::Admin { status, message }) => {
                assert_eq!(status, 500);
                assert_eq!(message, "boom");
            }
            other => panic!("expected an admin error, got {other:?}"),
        }
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn auth_proxy_client_revoke_run_returns_count() {
    timeout(Duration::from_secs(10), async {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/admin/v1/runs/run%201/tokens"))
            .and(header(
                "authorization",
                format!("Bearer {ADMIN_KEY}").as_str(),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"revoked": 3})))
            .mount(&server)
            .await;

        let client = AuthProxyClient::new(&server.uri(), ADMIN_KEY);
        assert_eq!(client.revoke_run("run 1").await.unwrap(), 3);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn auth_proxy_client_transport_error_does_not_leak_body() {
    timeout(Duration::from_secs(10), async {
        // Bind then drop: nothing listens on the port any more.
        let port = {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap().port()
        };
        let client = AuthProxyClient::new(&format!("http://127.0.0.1:{port}"), ADMIN_KEY);
        let err = client.issue(&request()).await.unwrap_err();
        assert!(matches!(err, AuthProxyError::Transport(_)), "{err}");
        let text = format!("{err} {err:?}");
        assert!(!text.contains(CREDENTIAL), "{text}");
        assert!(!text.contains(ADMIN_KEY), "{text}");
        assert!(!text.contains("127.0.0.1"), "{text}");
    })
    .await
    .expect("test timed out");
}
