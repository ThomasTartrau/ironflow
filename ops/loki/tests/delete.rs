use std::sync::Arc;

use ironflow_core::error::OperationError;
use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_loki::LokiClient;
use ironflow_ops_loki::delete::{CancelDeleteRequest, CreateDeleteRequest, ListDeleteRequests};
use serde_json::json;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn ctx() -> OperationContext {
    OperationContext::new(Arc::new(NoopSecretResolver))
}

#[tokio::test]
async fn create_delete_request_sends_post_with_query_params() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/loki/api/v1/delete"))
        .and(query_param("query", r#"{job="test"}"#))
        .and(query_param("start", "2024-01-01T00:00:00Z"))
        .and(query_param("end", "2024-01-02T00:00:00Z"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let loki = LokiClient::new(&server.uri(), reqwest::Client::new());
    let op = CreateDeleteRequest::new(
        loki,
        r#"{job="test"}"#,
        "2024-01-01T00:00:00Z",
        "2024-01-02T00:00:00Z",
    );

    let result = op.execute(&ctx()).await.unwrap();
    assert_eq!(result["status"], "success");
}

#[tokio::test]
async fn list_delete_requests_returns_entries() {
    let server = MockServer::start().await;
    let body = json!([
        {"request_id": "req-1", "status": "received"},
        {"request_id": "req-2", "status": "processed"}
    ]);

    Mock::given(method("GET"))
        .and(path("/loki/api/v1/delete"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let loki = LokiClient::new(&server.uri(), reqwest::Client::new());
    let op = ListDeleteRequests::new(loki);

    let result = op.execute(&ctx()).await.unwrap();
    let entries = result.as_array().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["request_id"], "req-1");
}

#[tokio::test]
async fn cancel_delete_request_sends_delete_with_request_id() {
    let server = MockServer::start().await;

    Mock::given(method("DELETE"))
        .and(path("/loki/api/v1/delete"))
        .and(query_param("request_id", "req-123"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let loki = LokiClient::new(&server.uri(), reqwest::Client::new());
    let op = CancelDeleteRequest::new(loki, "req-123");

    let result = op.execute(&ctx()).await.unwrap();
    assert_eq!(result["status"], "success");
}

#[tokio::test]
async fn create_delete_request_with_error_returns_operation_error() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/loki/api/v1/delete"))
        .respond_with(ResponseTemplate::new(400).set_body_string("invalid query"))
        .mount(&server)
        .await;

    let loki = LokiClient::new(&server.uri(), reqwest::Client::new());
    let op = CreateDeleteRequest::new(loki, "bad", "s", "e");

    let err = op.execute(&ctx()).await.unwrap_err();
    match err {
        OperationError::Http { status, message } => {
            assert_eq!(status, Some(400));
            assert_eq!(message, "invalid query");
        }
        other => panic!("expected Http error, got: {other}"),
    }
}
