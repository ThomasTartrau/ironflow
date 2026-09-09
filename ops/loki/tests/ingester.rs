use std::sync::Arc;

use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_loki::LokiClient;
use ironflow_ops_loki::ingester::{CancelShutdown, Flush, PrepareShutdown, Shutdown};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn ctx() -> OperationContext {
    OperationContext::new(Arc::new(NoopSecretResolver))
}

#[tokio::test]
async fn flush_sends_post() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/flush"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let loki = LokiClient::new(&server.uri(), reqwest::Client::new());
    let op = Flush::new(loki);

    let result = op.execute(&ctx()).await.unwrap();
    assert_eq!(result["status"], "success");
}

#[tokio::test]
async fn prepare_shutdown_sends_post_not_get() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/ingester/prepare_shutdown"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    // GET must not match -- only POST is mounted
    Mock::given(method("GET"))
        .and(path("/ingester/prepare_shutdown"))
        .respond_with(ResponseTemplate::new(405).set_body_string("method not allowed"))
        .mount(&server)
        .await;

    let loki = LokiClient::new(&server.uri(), reqwest::Client::new());
    let op = PrepareShutdown::new(loki);

    let result = op.execute(&ctx()).await.unwrap();
    assert_eq!(result["status"], "success");
}

#[tokio::test]
async fn cancel_shutdown_sends_delete() {
    let server = MockServer::start().await;

    Mock::given(method("DELETE"))
        .and(path("/ingester/prepare_shutdown"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let loki = LokiClient::new(&server.uri(), reqwest::Client::new());
    let op = CancelShutdown::new(loki);

    let result = op.execute(&ctx()).await.unwrap();
    assert_eq!(result["status"], "success");
}

#[tokio::test]
async fn shutdown_sends_post() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/ingester/shutdown"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let loki = LokiClient::new(&server.uri(), reqwest::Client::new());
    let op = Shutdown::new(loki);

    let result = op.execute(&ctx()).await.unwrap();
    assert_eq!(result["status"], "success");
}

#[tokio::test]
async fn flush_with_503_returns_error() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/flush"))
        .respond_with(ResponseTemplate::new(503).set_body_string("service unavailable"))
        .mount(&server)
        .await;

    let loki = LokiClient::new(&server.uri(), reqwest::Client::new());
    let op = Flush::new(loki);

    let err = op.execute(&ctx()).await.unwrap_err();
    match err {
        ironflow_core::error::OperationError::Http { status, message } => {
            assert_eq!(status, Some(503));
            assert_eq!(message, "service unavailable");
        }
        other => panic!("expected Http error, got: {other}"),
    }
}
