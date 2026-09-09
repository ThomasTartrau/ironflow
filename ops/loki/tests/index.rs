use std::sync::Arc;

use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_loki::LokiClient;
use ironflow_ops_loki::index::{GetIndexStats, GetIndexVolume, GetIndexVolumeRange};
use serde_json::json;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn ctx() -> OperationContext {
    OperationContext::new(Arc::new(NoopSecretResolver))
}

#[tokio::test]
async fn get_index_stats_returns_stats() {
    let server = MockServer::start().await;
    let body = json!({"streams": 42, "chunks": 1000, "entries": 50000, "bytes": 1048576});

    Mock::given(method("GET"))
        .and(path("/loki/api/v1/index/stats"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let loki = LokiClient::new(&server.uri(), reqwest::Client::new());
    let op = GetIndexStats::new(loki);

    let result = op.execute(&ctx()).await.unwrap();
    assert_eq!(result["streams"], 42);
    assert_eq!(result["bytes"], 1048576);
}

#[tokio::test]
async fn get_index_volume_sends_query_param() {
    let server = MockServer::start().await;
    let body = json!({"status": "success", "data": {"volumes": []}});

    Mock::given(method("GET"))
        .and(path("/loki/api/v1/index/volume"))
        .and(query_param("query", r#"{job="varlogs"}"#))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let loki = LokiClient::new(&server.uri(), reqwest::Client::new());
    let op = GetIndexVolume::new(loki, r#"{job="varlogs"}"#);

    let result = op.execute(&ctx()).await.unwrap();
    assert_eq!(result["status"], "success");
}

#[tokio::test]
async fn get_index_volume_range_sends_all_required_params() {
    let server = MockServer::start().await;
    let body = json!({"status": "success", "data": {"volumes": []}});

    Mock::given(method("GET"))
        .and(path("/loki/api/v1/index/volume_range"))
        .and(query_param("query", r#"{job="app"}"#))
        .and(query_param("start", "2024-01-01T00:00:00Z"))
        .and(query_param("end", "2024-01-02T00:00:00Z"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let loki = LokiClient::new(&server.uri(), reqwest::Client::new());
    let op = GetIndexVolumeRange::new(
        loki,
        r#"{job="app"}"#,
        "2024-01-01T00:00:00Z",
        "2024-01-02T00:00:00Z",
    );

    let result = op.execute(&ctx()).await.unwrap();
    assert_eq!(result["status"], "success");
}
