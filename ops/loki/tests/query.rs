use std::sync::Arc;

use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_loki::LokiClient;
use ironflow_ops_loki::query::{QueryInstant, QueryRange};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn ctx() -> OperationContext {
    OperationContext::new(Arc::new(NoopSecretResolver))
}

#[tokio::test]
async fn query_range_returns_logs() {
    let server = MockServer::start().await;
    let body = serde_json::json!({
        "status": "success",
        "data": {
            "resultType": "streams",
            "result": [{
                "stream": {"job": "varlogs"},
                "values": [["1234567890000000000", "hello world"]]
            }]
        }
    });

    Mock::given(method("GET"))
        .and(path("/loki/api/v1/query_range"))
        .and(query_param("query", r#"{job="varlogs"}"#))
        .and(query_param("start", "2024-01-01T00:00:00Z"))
        .and(query_param("end", "2024-01-02T00:00:00Z"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let loki = LokiClient::new(&server.uri(), reqwest::Client::new());
    let op = QueryRange::new(
        loki,
        r#"{job="varlogs"}"#,
        "2024-01-01T00:00:00Z",
        "2024-01-02T00:00:00Z",
    );

    let result = op.execute(&ctx()).await.unwrap();
    assert_eq!(result["status"], "success");
    assert_eq!(result["data"]["resultType"], "streams");
    assert_eq!(result["data"]["result"][0]["stream"]["job"], "varlogs");
}

#[tokio::test]
async fn query_instant_returns_result() {
    let server = MockServer::start().await;
    let body = serde_json::json!({
        "status": "success",
        "data": {
            "resultType": "vector",
            "result": [{
                "metric": {"job": "varlogs"},
                "value": [1234567890, "42"]
            }]
        }
    });

    Mock::given(method("GET"))
        .and(path("/loki/api/v1/query"))
        .and(query_param("query", r#"{job="varlogs"}"#))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let loki = LokiClient::new(&server.uri(), reqwest::Client::new());
    let op = QueryInstant::new(loki, r#"{job="varlogs"}"#);

    let result = op.execute(&ctx()).await.unwrap();
    assert_eq!(result["status"], "success");
    assert_eq!(result["data"]["resultType"], "vector");
}

#[tokio::test]
async fn query_range_with_http_error_returns_operation_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/loki/api/v1/query_range"))
        .respond_with(ResponseTemplate::new(400).set_body_string("bad query"))
        .mount(&server)
        .await;

    let loki = LokiClient::new(&server.uri(), reqwest::Client::new());
    let op = QueryRange::new(
        loki,
        "invalid",
        "2024-01-01T00:00:00Z",
        "2024-01-02T00:00:00Z",
    );

    let err = op.execute(&ctx()).await.unwrap_err();
    match err {
        ironflow_core::error::OperationError::Http { status, message } => {
            assert_eq!(status, Some(400));
            assert_eq!(message, "bad query");
        }
        other => panic!("expected Http error, got: {other}"),
    }
}

#[tokio::test]
async fn query_instant_sends_optional_params() {
    let server = MockServer::start().await;
    let body =
        serde_json::json!({"status": "success", "data": {"resultType": "vector", "result": []}});

    Mock::given(method("GET"))
        .and(path("/loki/api/v1/query"))
        .and(query_param("query", r#"{job="app"}"#))
        .and(query_param("time", "2024-06-01T00:00:00Z"))
        .and(query_param("limit", "10"))
        .and(query_param("direction", "backward"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let loki = LokiClient::new(&server.uri(), reqwest::Client::new());
    let op = QueryInstant::new(loki, r#"{job="app"}"#)
        .time("2024-06-01T00:00:00Z")
        .limit(10)
        .direction("backward");

    let result = op.execute(&ctx()).await.unwrap();
    assert_eq!(result["status"], "success");
}

#[tokio::test]
async fn query_range_sends_optional_step_and_limit() {
    let server = MockServer::start().await;
    let body =
        serde_json::json!({"status": "success", "data": {"resultType": "matrix", "result": []}});

    Mock::given(method("GET"))
        .and(path("/loki/api/v1/query_range"))
        .and(query_param("query", r#"{job="app"}"#))
        .and(query_param("step", "5m"))
        .and(query_param("limit", "100"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let loki = LokiClient::new(&server.uri(), reqwest::Client::new());
    let op = QueryRange::new(
        loki,
        r#"{job="app"}"#,
        "2024-01-01T00:00:00Z",
        "2024-01-02T00:00:00Z",
    )
    .step("5m")
    .limit(100);

    let result = op.execute(&ctx()).await.unwrap();
    assert_eq!(result["status"], "success");
}
