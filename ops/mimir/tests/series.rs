use std::sync::Arc;

use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_mimir::MimirClient;
use ironflow_ops_mimir::series::{
    GetActiveSeries, GetLabelValues, GetLabels, GetMetadata, GetSeries,
};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn ctx() -> OperationContext {
    OperationContext::new(Arc::new(NoopSecretResolver))
}

#[tokio::test]
async fn get_series_returns_json() {
    let server = MockServer::start().await;
    let body = serde_json::json!({
        "status": "success",
        "data": [{"__name__": "up", "job": "mimir"}]
    });

    Mock::given(method("GET"))
        .and(path("/prometheus/api/v1/series"))
        .and(query_param("match[]", "up"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = GetSeries::new(mimir, vec!["up".into()])
        .execute(&ctx())
        .await
        .unwrap();
    assert_eq!(result["status"], "success");
}

#[tokio::test]
async fn get_series_sends_optional_start_end() {
    let server = MockServer::start().await;
    let body = serde_json::json!({"status": "success", "data": []});

    Mock::given(method("GET"))
        .and(path("/prometheus/api/v1/series"))
        .and(query_param("start", "2024-01-01T00:00:00Z"))
        .and(query_param("end", "2024-01-02T00:00:00Z"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = GetSeries::new(mimir, vec!["up".into()])
        .start("2024-01-01T00:00:00Z")
        .end("2024-01-02T00:00:00Z")
        .execute(&ctx())
        .await
        .unwrap();
    assert_eq!(result["status"], "success");
}

#[tokio::test]
async fn get_labels_returns_json() {
    let server = MockServer::start().await;
    let body = serde_json::json!({
        "status": "success",
        "data": ["__name__", "job", "instance"]
    });

    Mock::given(method("GET"))
        .and(path("/prometheus/api/v1/labels"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = GetLabels::new(mimir).execute(&ctx()).await.unwrap();
    assert_eq!(result["status"], "success");
    assert_eq!(result["data"][0], "__name__");
}

#[tokio::test]
async fn get_labels_sends_optional_start_end() {
    let server = MockServer::start().await;
    let body = serde_json::json!({"status": "success", "data": []});

    Mock::given(method("GET"))
        .and(path("/prometheus/api/v1/labels"))
        .and(query_param("start", "2024-06-01T00:00:00Z"))
        .and(query_param("end", "2024-06-02T00:00:00Z"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = GetLabels::new(mimir)
        .start("2024-06-01T00:00:00Z")
        .end("2024-06-02T00:00:00Z")
        .execute(&ctx())
        .await
        .unwrap();
    assert_eq!(result["status"], "success");
}

#[tokio::test]
async fn get_label_values_returns_json() {
    let server = MockServer::start().await;
    let body = serde_json::json!({
        "status": "success",
        "data": ["mimir", "prometheus"]
    });

    Mock::given(method("GET"))
        .and(path("/prometheus/api/v1/label/job/values"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = GetLabelValues::new(mimir, "job")
        .execute(&ctx())
        .await
        .unwrap();
    assert_eq!(result["status"], "success");
    assert_eq!(result["data"][0], "mimir");
}

#[tokio::test]
async fn get_label_values_with_http_error() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/prometheus/api/v1/label/bad/values"))
        .respond_with(ResponseTemplate::new(400).set_body_string("invalid label"))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let err = GetLabelValues::new(mimir, "bad")
        .execute(&ctx())
        .await
        .unwrap_err();
    match err {
        ironflow_core::error::OperationError::Http { status, message } => {
            assert_eq!(status, Some(400));
            assert_eq!(message, "invalid label");
        }
        other => panic!("expected Http error, got: {other}"),
    }
}

#[tokio::test]
async fn get_metadata_returns_json() {
    let server = MockServer::start().await;
    let body = serde_json::json!({
        "status": "success",
        "data": {"up": [{"type": "gauge", "help": "Target up", "unit": ""}]}
    });

    Mock::given(method("GET"))
        .and(path("/prometheus/api/v1/metadata"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = GetMetadata::new(mimir).execute(&ctx()).await.unwrap();
    assert_eq!(result["status"], "success");
}

#[tokio::test]
async fn get_metadata_sends_optional_metric_and_limit() {
    let server = MockServer::start().await;
    let body = serde_json::json!({"status": "success", "data": {}});

    Mock::given(method("GET"))
        .and(path("/prometheus/api/v1/metadata"))
        .and(query_param("metric", "up"))
        .and(query_param("limit", "10"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = GetMetadata::new(mimir)
        .metric("up")
        .limit(10)
        .execute(&ctx())
        .await
        .unwrap();
    assert_eq!(result["status"], "success");
}

#[tokio::test]
async fn get_active_series_returns_json() {
    let server = MockServer::start().await;
    let body = serde_json::json!({
        "status": "success",
        "data": [{"__name__": "up"}]
    });

    Mock::given(method("GET"))
        .and(path("/prometheus/api/v1/cardinality/active_series"))
        .and(query_param("selector", "up"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = GetActiveSeries::new(mimir, "up")
        .execute(&ctx())
        .await
        .unwrap();
    assert_eq!(result["status"], "success");
}
