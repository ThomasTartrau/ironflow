use std::sync::Arc;

use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_mimir::MimirClient;
use ironflow_ops_mimir::cardinality::{GetLabelNamesCardinality, GetLabelValuesCardinality};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn ctx() -> OperationContext {
    OperationContext::new(Arc::new(NoopSecretResolver))
}

#[tokio::test]
async fn get_label_names_cardinality_returns_json() {
    let server = MockServer::start().await;
    let body = serde_json::json!({
        "status": "success",
        "data": [{"label_name": "__name__", "label_values_count": 42}]
    });

    Mock::given(method("GET"))
        .and(path("/prometheus/api/v1/cardinality/label_names"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = GetLabelNamesCardinality::new(mimir)
        .execute(&ctx())
        .await
        .unwrap();
    assert_eq!(result["status"], "success");
}

#[tokio::test]
async fn get_label_names_cardinality_sends_optional_selector_and_limit() {
    let server = MockServer::start().await;
    let body = serde_json::json!({"status": "success", "data": []});

    Mock::given(method("GET"))
        .and(path("/prometheus/api/v1/cardinality/label_names"))
        .and(query_param("selector", "{job=\"mimir\"}"))
        .and(query_param("limit", "5"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = GetLabelNamesCardinality::new(mimir)
        .selector("{job=\"mimir\"}")
        .limit(5)
        .execute(&ctx())
        .await
        .unwrap();
    assert_eq!(result["status"], "success");
}

#[tokio::test]
async fn get_label_values_cardinality_returns_json() {
    let server = MockServer::start().await;
    let body = serde_json::json!({
        "status": "success",
        "data": [{"label_value": "up", "series_count": 10}]
    });

    Mock::given(method("GET"))
        .and(path("/prometheus/api/v1/cardinality/label_values"))
        .and(query_param("label_names[]", "__name__"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = GetLabelValuesCardinality::new(mimir, "__name__")
        .execute(&ctx())
        .await
        .unwrap();
    assert_eq!(result["status"], "success");
}

#[tokio::test]
async fn get_label_values_cardinality_sends_optional_selector_and_limit() {
    let server = MockServer::start().await;
    let body = serde_json::json!({"status": "success", "data": []});

    Mock::given(method("GET"))
        .and(path("/prometheus/api/v1/cardinality/label_values"))
        .and(query_param("label_names[]", "__name__"))
        .and(query_param("selector", "{job=\"mimir\"}"))
        .and(query_param("limit", "3"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = GetLabelValuesCardinality::new(mimir, "__name__")
        .selector("{job=\"mimir\"}")
        .limit(3)
        .execute(&ctx())
        .await
        .unwrap();
    assert_eq!(result["status"], "success");
}

#[tokio::test]
async fn get_label_names_cardinality_with_http_error() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/prometheus/api/v1/cardinality/label_names"))
        .respond_with(ResponseTemplate::new(500).set_body_string("internal error"))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let err = GetLabelNamesCardinality::new(mimir)
        .execute(&ctx())
        .await
        .unwrap_err();
    match err {
        ironflow_core::error::OperationError::Http { status, message } => {
            assert_eq!(status, Some(500));
            assert_eq!(message, "internal error");
        }
        other => panic!("expected Http error, got: {other}"),
    }
}
