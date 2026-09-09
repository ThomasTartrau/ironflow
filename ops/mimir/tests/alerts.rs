use std::sync::Arc;

use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_mimir::MimirClient;
use ironflow_ops_mimir::alerts::{
    DeleteAlertmanagerConfig, GetAlertmanagerConfig, GetAlertmanagerConfigs, GetAlertmanagerStatus,
    GetAlerts, SetAlertmanagerConfig,
};
use wiremock::matchers::{body_string, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn ctx() -> OperationContext {
    OperationContext::new(Arc::new(NoopSecretResolver))
}

#[tokio::test]
async fn get_alerts_returns_json() {
    let server = MockServer::start().await;
    let body = serde_json::json!({
        "status": "success",
        "data": {"alerts": []}
    });

    Mock::given(method("GET"))
        .and(path("/prometheus/api/v1/alerts"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = GetAlerts::new(mimir).execute(&ctx()).await.unwrap();
    assert_eq!(result["status"], "success");
}

#[tokio::test]
async fn get_alertmanager_config_returns_json() {
    let server = MockServer::start().await;
    let body = serde_json::json!({
        "template_files": {},
        "alertmanager_config": "route:\n  receiver: default"
    });

    Mock::given(method("GET"))
        .and(path("/api/v1/alerts"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = GetAlertmanagerConfig::new(mimir)
        .execute(&ctx())
        .await
        .unwrap();
    assert!(result["alertmanager_config"].is_string());
}

#[tokio::test]
async fn set_alertmanager_config_sends_yaml() {
    let server = MockServer::start().await;
    let config = "route:\n  receiver: default";

    Mock::given(method("POST"))
        .and(path("/api/v1/alerts"))
        .and(header("Content-Type", "application/yaml"))
        .and(body_string(config))
        .respond_with(ResponseTemplate::new(201))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = SetAlertmanagerConfig::new(mimir, config)
        .execute(&ctx())
        .await
        .unwrap();
    assert_eq!(result["status"], "success");
}

#[tokio::test]
async fn delete_alertmanager_config_sends_delete() {
    let server = MockServer::start().await;

    Mock::given(method("DELETE"))
        .and(path("/api/v1/alerts"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = DeleteAlertmanagerConfig::new(mimir)
        .execute(&ctx())
        .await
        .unwrap();
    assert_eq!(result["status"], "success");
}

#[tokio::test]
async fn get_alerts_http_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/prometheus/api/v1/alerts"))
        .respond_with(ResponseTemplate::new(500).set_body_string("internal error"))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let err = GetAlerts::new(mimir).execute(&ctx()).await.unwrap_err();
    match err {
        ironflow_core::error::OperationError::Http { status, message } => {
            assert_eq!(status, Some(500));
            assert_eq!(message, "internal error");
        }
        other => panic!("expected Http error, got: {other}"),
    }
}

#[tokio::test]
async fn get_alertmanager_status_returns_json() {
    let server = MockServer::start().await;
    let body = serde_json::json!({
        "cluster": {"status": "ready", "peers": 3}
    });

    Mock::given(method("GET"))
        .and(path("/multitenant_alertmanager/status"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = GetAlertmanagerStatus::new(mimir)
        .execute(&ctx())
        .await
        .unwrap();
    assert_eq!(result["cluster"]["status"], "ready");
}

#[tokio::test]
async fn get_alertmanager_configs_returns_json() {
    let server = MockServer::start().await;
    let body = serde_json::json!({
        "tenant-1": {"alertmanager_config": "route:\n  receiver: default"},
        "tenant-2": {"alertmanager_config": "route:\n  receiver: slack"}
    });

    Mock::given(method("GET"))
        .and(path("/multitenant_alertmanager/configs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = GetAlertmanagerConfigs::new(mimir)
        .execute(&ctx())
        .await
        .unwrap();
    assert!(result["tenant-1"]["alertmanager_config"].is_string());
}
