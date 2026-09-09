use std::sync::Arc;

use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_mimir::MimirClient;
use ironflow_ops_mimir::ingest::{InfluxWrite, OtlpMetricsWrite, RemoteWrite};
use serde_json::json;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn ctx() -> OperationContext {
    OperationContext::new(Arc::new(NoopSecretResolver))
}

#[tokio::test]
async fn remote_write_sends_protobuf_headers() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/v1/push"))
        .and(header("Content-Type", "application/x-protobuf"))
        .and(header("Content-Encoding", "snappy"))
        .and(header("X-Prometheus-Remote-Write-Version", "0.1.0"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = RemoteWrite::new(mimir, vec![1, 2, 3])
        .execute(&ctx())
        .await
        .unwrap();
    assert_eq!(result["status"], "success");
}

#[tokio::test]
async fn otlp_metrics_write_sends_json() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/otlp/v1/metrics"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = OtlpMetricsWrite::new(mimir, json!({"resourceMetrics": []}))
        .execute(&ctx())
        .await
        .unwrap();
    assert_eq!(result["status"], "success");
}

#[tokio::test]
async fn influx_write_sends_line_protocol() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/v1/push/influx/write"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = InfluxWrite::new(mimir, "cpu,host=A value=0.5")
        .execute(&ctx())
        .await
        .unwrap();
    assert_eq!(result["status"], "success");
}

#[tokio::test]
async fn remote_write_with_error_returns_operation_error() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/v1/push"))
        .respond_with(ResponseTemplate::new(400).set_body_string("invalid samples"))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let err = RemoteWrite::new(mimir, vec![1, 2, 3])
        .execute(&ctx())
        .await
        .unwrap_err();
    match err {
        ironflow_core::error::OperationError::Http { status, message } => {
            assert_eq!(status, Some(400));
            assert_eq!(message, "invalid samples");
        }
        other => panic!("expected Http error, got: {other}"),
    }
}
