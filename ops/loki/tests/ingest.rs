use std::sync::Arc;

use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_loki::LokiClient;
use ironflow_ops_loki::ingest::{PushLogs, PushLogsOtlp};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn ctx() -> OperationContext {
    OperationContext::new(Arc::new(NoopSecretResolver))
}

#[tokio::test]
async fn push_logs_sends_payload_and_returns_success() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/loki/api/v1/push"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let loki = LokiClient::new(&server.uri(), reqwest::Client::new());
    let payload = json!({
        "streams": [{
            "stream": { "job": "test" },
            "values": [["1234567890000000000", "hello world"]]
        }]
    });
    let op = PushLogs::new(loki, payload);

    let result = op.execute(&ctx()).await.unwrap();
    assert_eq!(result["status"], "success");
}

#[tokio::test]
async fn push_logs_with_server_error_returns_operation_error() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/loki/api/v1/push"))
        .respond_with(ResponseTemplate::new(500).set_body_string("internal error"))
        .mount(&server)
        .await;

    let loki = LokiClient::new(&server.uri(), reqwest::Client::new());
    let op = PushLogs::new(loki, json!({"streams": []}));

    let err = op.execute(&ctx()).await.unwrap_err();
    match err {
        ironflow_core::error::OperationError::Http { status, message } => {
            assert_eq!(status, Some(500));
            assert_eq!(message, "internal error");
        }
        other => panic!("expected Http error, got: {other}"),
    }
}

#[tokio::test]
async fn push_logs_otlp_sends_to_otlp_endpoint() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/otlp/v1/logs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"partialSuccess": {}})))
        .mount(&server)
        .await;

    let loki = LokiClient::new(&server.uri(), reqwest::Client::new());
    let payload = json!({"resourceLogs": [{"scopeLogs": []}]});
    let op = PushLogsOtlp::new(loki, payload);

    let result = op.execute(&ctx()).await.unwrap();
    assert!(result.get("partialSuccess").is_some());
}
