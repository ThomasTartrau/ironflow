use std::sync::Arc;

use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_mimir::MimirClient;
use ironflow_ops_mimir::query::{FormatQuery, QueryExemplars, QueryInstant, QueryRange};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn ctx() -> OperationContext {
    OperationContext::new(Arc::new(NoopSecretResolver))
}

#[tokio::test]
async fn query_instant_returns_result() {
    let server = MockServer::start().await;
    let body = serde_json::json!({
        "status": "success",
        "data": {
            "resultType": "vector",
            "result": [{
                "metric": {"__name__": "up", "job": "mimir"},
                "value": [1234567890, "1"]
            }]
        }
    });

    Mock::given(method("GET"))
        .and(path("/prometheus/api/v1/query"))
        .and(query_param("query", "up"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let op = QueryInstant::new(mimir, "up");

    let result = op.execute(&ctx()).await.unwrap();
    assert_eq!(result["status"], "success");
    assert_eq!(result["data"]["resultType"], "vector");
}

#[tokio::test]
async fn query_range_returns_metrics() {
    let server = MockServer::start().await;
    let body = serde_json::json!({
        "status": "success",
        "data": {
            "resultType": "matrix",
            "result": [{
                "metric": {"__name__": "up"},
                "values": [[1234567890, "1"], [1234567891, "1"]]
            }]
        }
    });

    Mock::given(method("GET"))
        .and(path("/prometheus/api/v1/query_range"))
        .and(query_param("query", "up"))
        .and(query_param("start", "2024-01-01T00:00:00Z"))
        .and(query_param("end", "2024-01-02T00:00:00Z"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let op = QueryRange::new(mimir, "up", "2024-01-01T00:00:00Z", "2024-01-02T00:00:00Z");

    let result = op.execute(&ctx()).await.unwrap();
    assert_eq!(result["status"], "success");
    assert_eq!(result["data"]["resultType"], "matrix");
    assert_eq!(result["data"]["result"][0]["metric"]["__name__"], "up");
}

#[tokio::test]
async fn query_range_with_http_error_returns_operation_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/prometheus/api/v1/query_range"))
        .respond_with(ResponseTemplate::new(400).set_body_string("bad query"))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let op = QueryRange::new(
        mimir,
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
    let body = serde_json::json!({
        "status": "success",
        "data": {"resultType": "vector", "result": []}
    });

    Mock::given(method("GET"))
        .and(path("/prometheus/api/v1/query"))
        .and(query_param("query", "up"))
        .and(query_param("time", "2024-06-01T00:00:00Z"))
        .and(query_param("timeout", "30s"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let op = QueryInstant::new(mimir, "up")
        .time("2024-06-01T00:00:00Z")
        .timeout("30s");

    let result = op.execute(&ctx()).await.unwrap();
    assert_eq!(result["status"], "success");
}

#[tokio::test]
async fn query_range_sends_optional_step() {
    let server = MockServer::start().await;
    let body = serde_json::json!({
        "status": "success",
        "data": {"resultType": "matrix", "result": []}
    });

    Mock::given(method("GET"))
        .and(path("/prometheus/api/v1/query_range"))
        .and(query_param("query", "up"))
        .and(query_param("step", "5m"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let op =
        QueryRange::new(mimir, "up", "2024-01-01T00:00:00Z", "2024-01-02T00:00:00Z").step("5m");

    let result = op.execute(&ctx()).await.unwrap();
    assert_eq!(result["status"], "success");
}

#[tokio::test]
async fn query_exemplars_returns_result() {
    let server = MockServer::start().await;
    let body = serde_json::json!({
        "status": "success",
        "data": [{
            "seriesLabels": {"__name__": "http_requests_total"},
            "exemplars": [{"labels": {"traceID": "abc"}, "value": "1", "timestamp": 1234567890.0}]
        }]
    });

    Mock::given(method("GET"))
        .and(path("/prometheus/api/v1/query_exemplars"))
        .and(query_param("query", "http_requests_total"))
        .and(query_param("start", "2024-01-01T00:00:00Z"))
        .and(query_param("end", "2024-01-02T00:00:00Z"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = QueryExemplars::new(
        mimir,
        "http_requests_total",
        "2024-01-01T00:00:00Z",
        "2024-01-02T00:00:00Z",
    )
    .execute(&ctx())
    .await
    .unwrap();
    assert_eq!(result["status"], "success");
    assert_eq!(
        result["data"][0]["seriesLabels"]["__name__"],
        "http_requests_total"
    );
}

#[tokio::test]
async fn format_query_returns_formatted() {
    let server = MockServer::start().await;
    let body = serde_json::json!({
        "status": "success",
        "data": "up == 1"
    });

    Mock::given(method("GET"))
        .and(path("/prometheus/api/v1/format_query"))
        .and(query_param("query", "up==1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = FormatQuery::new(mimir, "up==1")
        .execute(&ctx())
        .await
        .unwrap();
    assert_eq!(result["status"], "success");
    assert_eq!(result["data"], "up == 1");
}
