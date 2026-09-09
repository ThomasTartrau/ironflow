//! Integration tests for [`HttpApiClient`].

use std::collections::HashMap;
use std::sync::Arc;

use ironflow_core::error::OperationError;
use ironflow_core::operation::{NoopSecretResolver, OperationContext};
use ironflow_ops_common::{Auth, HttpApiClient, MapSecretResolver};
use reqwest::Client;
use wiremock::matchers::{header, header_exists, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

// -- Row 1: Construction and auth bearer/basic/none --

#[test]
fn new_trims_trailing_slash() {
    let client = HttpApiClient::new("http://api.example.com/", Client::new());
    assert_eq!(client.base_url(), "http://api.example.com");
}

#[test]
fn new_preserves_url_without_trailing_slash() {
    let client = HttpApiClient::new("http://api.example.com", Client::new());
    assert_eq!(client.base_url(), "http://api.example.com");
}

#[test]
fn with_bearer_token_sets_auth() {
    let client =
        HttpApiClient::new("http://api.example.com", Client::new()).with_bearer_token("tok");
    assert!(matches!(client.auth(), Auth::Bearer(t) if t == "tok"));
}

#[test]
fn with_basic_auth_sets_auth() {
    let client =
        HttpApiClient::new("http://api.example.com", Client::new()).with_basic_auth("user", "pass");
    assert!(
        matches!(client.auth(), Auth::Basic { user, password } if user == "user" && password == "pass")
    );
}

#[tokio::test]
async fn bearer_auth_sends_authorization_header() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(header("authorization", "Bearer test-token-123"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .mount(&server)
        .await;

    let client =
        HttpApiClient::new(&server.uri(), Client::new()).with_bearer_token("test-token-123");
    let resp = client.get("/check").send().await.unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn basic_auth_sends_authorization_header() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(header_exists("authorization"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .mount(&server)
        .await;

    let client =
        HttpApiClient::new(&server.uri(), Client::new()).with_basic_auth("admin", "secret");
    let resp = client.get("/check").send().await.unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn no_auth_sends_no_authorization_header() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .mount(&server)
        .await;

    let client = HttpApiClient::new(&server.uri(), Client::new());
    let resp = client.get("/check").send().await.unwrap();
    assert_eq!(resp.status(), 200);

    let requests = server.received_requests().await.unwrap();
    assert!(
        !requests[0].headers.contains_key("authorization"),
        "no-auth client must not send authorization header"
    );
}

// -- Row 2: from_context with secrets --

#[tokio::test]
async fn from_context_fails_without_url() {
    let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
    let err = HttpApiClient::from_context(&ctx, "api_url", "api_token", "api_basic_auth")
        .await
        .unwrap_err();
    match err {
        OperationError::Secret { message } => {
            assert!(
                message.contains("api_url"),
                "expected api_url error, got: {message}"
            );
        }
        other => panic!("expected Secret error, got: {other}"),
    }
}

#[tokio::test]
async fn from_context_with_bearer_token() {
    let mut secrets = HashMap::new();
    secrets.insert("api_url".into(), "http://api.example.com".into());
    secrets.insert("api_token".into(), "my-token".into());
    let ctx = OperationContext::new(Arc::new(MapSecretResolver::new(secrets)));
    let client = HttpApiClient::from_context(&ctx, "api_url", "api_token", "api_basic_auth")
        .await
        .unwrap();
    assert_eq!(client.base_url(), "http://api.example.com");
    assert!(matches!(client.auth(), Auth::Bearer(t) if t == "my-token"));
}

#[tokio::test]
async fn from_context_with_basic_auth() {
    let mut secrets = HashMap::new();
    secrets.insert("api_url".into(), "http://api.example.com".into());
    secrets.insert("api_basic_auth".into(), "admin:secret".into());
    let ctx = OperationContext::new(Arc::new(MapSecretResolver::new(secrets)));
    let client = HttpApiClient::from_context(&ctx, "api_url", "api_token", "api_basic_auth")
        .await
        .unwrap();
    assert!(
        matches!(client.auth(), Auth::Basic { user, password } if user == "admin" && password == "secret")
    );
}

#[tokio::test]
async fn from_context_rejects_basic_auth_without_colon() {
    let mut secrets = HashMap::new();
    secrets.insert("api_url".into(), "http://api.example.com".into());
    secrets.insert("api_basic_auth".into(), "no-colon-here".into());
    let ctx = OperationContext::new(Arc::new(MapSecretResolver::new(secrets)));
    let err = HttpApiClient::from_context(&ctx, "api_url", "api_token", "api_basic_auth")
        .await
        .unwrap_err();
    match err {
        OperationError::Secret { message } => {
            assert!(
                message.contains("user:password"),
                "expected format hint, got: {message}"
            );
        }
        other => panic!("expected Secret error, got: {other}"),
    }
}

#[tokio::test]
async fn from_context_no_auth() {
    let mut secrets = HashMap::new();
    secrets.insert("api_url".into(), "http://api.example.com".into());
    let ctx = OperationContext::new(Arc::new(MapSecretResolver::new(secrets)));
    let client = HttpApiClient::from_context(&ctx, "api_url", "api_token", "api_basic_auth")
        .await
        .unwrap();
    assert!(matches!(client.auth(), Auth::None));
}

// -- Row 7: Debug does not leak secrets --

#[test]
fn debug_does_not_leak_bearer_token() {
    let client = HttpApiClient::new("http://api.example.com", Client::new())
        .with_bearer_token("super-secret-token");
    let debug = format!("{client:?}");
    assert!(
        !debug.contains("super-secret-token"),
        "Debug output must not contain the bearer token: {debug}"
    );
    assert!(debug.contains("<redacted>"));
}

#[test]
fn debug_does_not_leak_password() {
    let client = HttpApiClient::new("http://api.example.com", Client::new())
        .with_basic_auth("admin", "super-secret-password");
    let debug = format!("{client:?}");
    assert!(
        !debug.contains("super-secret-password"),
        "Debug output must not contain the password: {debug}"
    );
    assert!(debug.contains("<redacted>"));
    assert!(debug.contains("admin"));
}

#[test]
fn url_builds_correct_path() {
    let client = HttpApiClient::new("http://api.example.com", Client::new());
    assert_eq!(
        client.url("/api/v1/status"),
        "http://api.example.com/api/v1/status"
    );
}
