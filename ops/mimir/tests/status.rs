use std::sync::Arc;

use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_mimir::MimirClient;
use ironflow_ops_mimir::status::{
    GetBuildInfo, GetConfig, GetConfigDiff, GetMetrics, GetReady, GetServices, GetUserLimits,
};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn ctx() -> OperationContext {
    OperationContext::new(Arc::new(NoopSecretResolver))
}

#[tokio::test]
async fn get_ready_returns_status() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/ready"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ready"))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = GetReady::new(mimir).execute(&ctx()).await.unwrap();
    assert_eq!(result["ready"], true);
    assert_eq!(result["status"], 200);
    assert_eq!(result["body"], "ready");
}

#[tokio::test]
async fn get_ready_not_ready_returns_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/ready"))
        .respond_with(ResponseTemplate::new(503).set_body_string("not ready"))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let err = GetReady::new(mimir).execute(&ctx()).await.unwrap_err();
    match err {
        ironflow_core::error::OperationError::Http { status, message } => {
            assert_eq!(status, Some(503));
            assert_eq!(message, "not ready");
        }
        other => panic!("expected Http error, got: {other}"),
    }
}

#[tokio::test]
async fn get_metrics_returns_text() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/metrics"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string("# HELP up\n# TYPE up gauge\nup 1"),
        )
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = GetMetrics::new(mimir).execute(&ctx()).await.unwrap();
    assert!(result["metrics"].as_str().unwrap().contains("up 1"));
}

#[tokio::test]
async fn get_config_returns_yaml() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/config"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string("server:\n  http_listen_port: 8080"),
        )
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = GetConfig::new(mimir).execute(&ctx()).await.unwrap();
    assert!(
        result["config"]
            .as_str()
            .unwrap()
            .contains("http_listen_port")
    );
}

#[tokio::test]
async fn get_config_diff_returns_text() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/config"))
        .and(wiremock::matchers::query_param("mode", "diff"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string("server:\n  http_listen_port: 9090"),
        )
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = GetConfigDiff::new(mimir).execute(&ctx()).await.unwrap();
    assert!(
        result["config_diff"]
            .as_str()
            .unwrap()
            .contains("http_listen_port")
    );
}

#[tokio::test]
async fn get_services_returns_text() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/services"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string("ingester: Running\ncompactor: Running"),
        )
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = GetServices::new(mimir).execute(&ctx()).await.unwrap();
    assert!(result["services"].as_str().unwrap().contains("ingester"));
}

#[tokio::test]
async fn get_build_info_returns_json() {
    let server = MockServer::start().await;
    let body = serde_json::json!({
        "status": "success",
        "data": {"version": "2.10.0", "revision": "abc123"}
    });

    Mock::given(method("GET"))
        .and(path("/api/v1/status/buildinfo"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = GetBuildInfo::new(mimir).execute(&ctx()).await.unwrap();
    assert_eq!(result["data"]["version"], "2.10.0");
}

#[tokio::test]
async fn get_user_limits_returns_text() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/runtime_config"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string("ingestion_rate: 10000\nmax_series: 100000"),
        )
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = GetUserLimits::new(mimir).execute(&ctx()).await.unwrap();
    assert!(
        result["limits"]
            .as_str()
            .unwrap()
            .contains("ingestion_rate")
    );
}
