use std::sync::Arc;

use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_loki::LokiClient;
use ironflow_ops_loki::labels::{GetLabelValues, GetLabels, GetSeries};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn ctx() -> OperationContext {
    OperationContext::new(Arc::new(NoopSecretResolver))
}

#[tokio::test]
async fn get_labels_returns_label_names() {
    let server = MockServer::start().await;
    let body = serde_json::json!({
        "status": "success",
        "data": ["job", "instance", "level"]
    });

    Mock::given(method("GET"))
        .and(path("/loki/api/v1/labels"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let loki = LokiClient::new(&server.uri(), reqwest::Client::new());
    let op = GetLabels::new(loki);
    let result = op.execute(&ctx()).await.unwrap();

    assert_eq!(result["status"], "success");
    let data = result["data"].as_array().unwrap();
    assert_eq!(data.len(), 3);
    assert_eq!(data[0], "job");
}

#[tokio::test]
async fn get_label_values_returns_values() {
    let server = MockServer::start().await;
    let body = serde_json::json!({
        "status": "success",
        "data": ["varlogs", "nginx", "app"]
    });

    Mock::given(method("GET"))
        .and(path("/loki/api/v1/label/job/values"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let loki = LokiClient::new(&server.uri(), reqwest::Client::new());
    let op = GetLabelValues::new(loki, "job");
    let result = op.execute(&ctx()).await.unwrap();

    assert_eq!(result["status"], "success");
    let data = result["data"].as_array().unwrap();
    assert!(data.contains(&serde_json::json!("varlogs")));
}

#[tokio::test]
async fn get_series_returns_matching_series() {
    let server = MockServer::start().await;
    let body = serde_json::json!({
        "status": "success",
        "data": [{"job": "varlogs", "instance": "localhost:3100"}]
    });

    Mock::given(method("GET"))
        .and(path("/loki/api/v1/series"))
        .and(query_param("match[]", r#"{job="varlogs"}"#))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let loki = LokiClient::new(&server.uri(), reqwest::Client::new());
    let op = GetSeries::new(loki, vec![r#"{job="varlogs"}"#.into()]);
    let result = op.execute(&ctx()).await.unwrap();

    assert_eq!(result["status"], "success");
    assert_eq!(result["data"][0]["job"], "varlogs");
}
