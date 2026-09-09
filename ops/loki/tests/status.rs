use std::sync::Arc;

use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_loki::LokiClient;
use ironflow_ops_loki::status::{GetConfig, GetLogLevel, GetMetrics, GetReady, SetLogLevel};
use serde_json::json;
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn ctx() -> OperationContext {
    OperationContext::new(Arc::new(NoopSecretResolver))
}

#[tokio::test]
async fn get_ready_returns_status_when_ready() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/ready"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ready"))
        .mount(&server)
        .await;

    let loki = LokiClient::new(&server.uri(), reqwest::Client::new());
    let op = GetReady::new(loki);

    let result = op.execute(&ctx()).await.unwrap();
    assert_eq!(result["ready"], true);
    assert_eq!(result["status"], 200);
    assert_eq!(result["body"], "ready");
}

#[tokio::test]
async fn get_ready_returns_error_when_not_ready() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/ready"))
        .respond_with(ResponseTemplate::new(503).set_body_string("not ready"))
        .mount(&server)
        .await;

    let loki = LokiClient::new(&server.uri(), reqwest::Client::new());
    let op = GetReady::new(loki);

    let err = op.execute(&ctx()).await.unwrap_err();
    match err {
        ironflow_core::error::OperationError::Http { status, message } => {
            assert_eq!(status, Some(503));
            assert_eq!(message, "not ready");
        }
        other => panic!("expected Http error, got: {other}"),
    }
}

#[tokio::test]
async fn get_log_level_returns_current_level() {
    let server = MockServer::start().await;
    let body = json!({"level": "info"});

    Mock::given(method("GET"))
        .and(path("/log_level"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let loki = LokiClient::new(&server.uri(), reqwest::Client::new());
    let op = GetLogLevel::new(loki);

    let result = op.execute(&ctx()).await.unwrap();
    assert_eq!(result["level"], "info");
}

#[tokio::test]
async fn set_log_level_sends_post_with_json_body() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/log_level"))
        .and(body_json(json!({"log_level": "debug"})))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let loki = LokiClient::new(&server.uri(), reqwest::Client::new());
    let op = SetLogLevel::new(loki, "debug");

    let result = op.execute(&ctx()).await.unwrap();
    assert_eq!(result["status"], "success");
}

#[tokio::test]
async fn get_metrics_returns_text_body() {
    let server = MockServer::start().await;
    let prometheus_text = "# HELP loki_ingester_chunks_flushed Total flushed chunks.\nloki_ingester_chunks_flushed 42\n";

    Mock::given(method("GET"))
        .and(path("/metrics"))
        .respond_with(ResponseTemplate::new(200).set_body_string(prometheus_text))
        .mount(&server)
        .await;

    let loki = LokiClient::new(&server.uri(), reqwest::Client::new());
    let op = GetMetrics::new(loki);

    let result = op.execute(&ctx()).await.unwrap();
    let metrics = result["metrics"].as_str().unwrap();
    assert!(metrics.contains("loki_ingester_chunks_flushed 42"));
}

#[tokio::test]
async fn get_config_returns_text_body() {
    let server = MockServer::start().await;
    let yaml_config = "auth_enabled: false\nserver:\n  http_listen_port: 3100\n";

    Mock::given(method("GET"))
        .and(path("/config"))
        .respond_with(ResponseTemplate::new(200).set_body_string(yaml_config))
        .mount(&server)
        .await;

    let loki = LokiClient::new(&server.uri(), reqwest::Client::new());
    let op = GetConfig::new(loki);

    let result = op.execute(&ctx()).await.unwrap();
    let config = result["config"].as_str().unwrap();
    assert!(config.contains("auth_enabled: false"));
}
