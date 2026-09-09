use std::sync::Arc;

use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_loki::LokiClient;
use ironflow_ops_loki::patterns::{DetectFields, DetectPatterns, GetDetectedFieldValues};
use serde_json::json;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn ctx() -> OperationContext {
    OperationContext::new(Arc::new(NoopSecretResolver))
}

#[tokio::test]
async fn detect_patterns_sends_query() {
    let server = MockServer::start().await;
    let body = json!({"status": "success", "data": [{"pattern": "<_> error <_>", "count": 42}]});

    Mock::given(method("GET"))
        .and(path("/loki/api/v1/patterns"))
        .and(query_param("query", r#"{job="varlogs"}"#))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let loki = LokiClient::new(&server.uri(), reqwest::Client::new());
    let op = DetectPatterns::new(loki, r#"{job="varlogs"}"#);

    let result = op.execute(&ctx()).await.unwrap();
    assert_eq!(result["status"], "success");
    assert_eq!(result["data"][0]["count"], 42);
}

#[tokio::test]
async fn detect_fields_sends_query_with_optional_params() {
    let server = MockServer::start().await;
    let body = json!({"status": "success", "data": [{"name": "level", "type": "string"}]});

    Mock::given(method("GET"))
        .and(path("/loki/api/v1/detected_fields"))
        .and(query_param("query", r#"{job="app"}"#))
        .and(query_param("start", "2024-01-01T00:00:00Z"))
        .and(query_param("field_limit", "10"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let loki = LokiClient::new(&server.uri(), reqwest::Client::new());
    let op = DetectFields::new(loki, r#"{job="app"}"#)
        .start("2024-01-01T00:00:00Z")
        .field_limit(10);

    let result = op.execute(&ctx()).await.unwrap();
    assert_eq!(result["data"][0]["name"], "level");
}

#[tokio::test]
async fn get_detected_field_values_sends_correct_path() {
    let server = MockServer::start().await;
    let body = json!({"status": "success", "data": ["error", "info", "debug"]});

    Mock::given(method("GET"))
        .and(path("/loki/api/v1/detected_field/level/values"))
        .and(query_param("query", r#"{job="app"}"#))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let loki = LokiClient::new(&server.uri(), reqwest::Client::new());
    let op = GetDetectedFieldValues::new(loki, "level", r#"{job="app"}"#);

    let result = op.execute(&ctx()).await.unwrap();
    let values = result["data"].as_array().unwrap();
    assert_eq!(values.len(), 3);
}
