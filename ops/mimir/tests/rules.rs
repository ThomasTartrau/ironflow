use std::sync::Arc;

use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_mimir::MimirClient;
use ironflow_ops_mimir::rules::{
    CreateRuleGroup, DeleteRuleGroup, DeleteRuleNamespace, GetAllTenantRules, GetRuleGroup,
    GetRules, GetRulesByNamespace,
};
use wiremock::matchers::{body_string, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn ctx() -> OperationContext {
    OperationContext::new(Arc::new(NoopSecretResolver))
}

#[tokio::test]
async fn get_rules_returns_json() {
    let server = MockServer::start().await;
    let body = serde_json::json!({
        "status": "success",
        "data": {"groups": []}
    });

    Mock::given(method("GET"))
        .and(path("/prometheus/api/v1/rules"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = GetRules::new(mimir).execute(&ctx()).await.unwrap();
    assert_eq!(result["status"], "success");
}

#[tokio::test]
async fn create_rule_group_sends_yaml() {
    let server = MockServer::start().await;
    let yaml = "name: test-group\ninterval: 1m\nrules: []";

    Mock::given(method("POST"))
        .and(path("/prometheus/config/v1/rules/production"))
        .and(header("Content-Type", "application/yaml"))
        .and(body_string(yaml))
        .respond_with(ResponseTemplate::new(202))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = CreateRuleGroup::new(mimir, "production", yaml)
        .execute(&ctx())
        .await
        .unwrap();
    assert_eq!(result["status"], "success");
}

#[tokio::test]
async fn delete_rule_group_sends_delete() {
    let server = MockServer::start().await;

    Mock::given(method("DELETE"))
        .and(path("/prometheus/config/v1/rules/production/cpu-alerts"))
        .respond_with(ResponseTemplate::new(202))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = DeleteRuleGroup::new(mimir, "production", "cpu-alerts")
        .execute(&ctx())
        .await
        .unwrap();
    assert_eq!(result["status"], "success");
}

#[tokio::test]
async fn get_all_tenant_rules_sends_admin_header() {
    let server = MockServer::start().await;
    let body = serde_json::json!({
        "status": "success",
        "data": {"groups": []}
    });

    Mock::given(method("GET"))
        .and(path("/prometheus/api/v1/rules"))
        .and(header("X-Scope-OrgID", "__admin__"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = GetAllTenantRules::new(mimir).execute(&ctx()).await.unwrap();
    assert_eq!(result["status"], "success");
}

#[tokio::test]
async fn get_rules_by_namespace_returns_json() {
    let server = MockServer::start().await;
    let body = serde_json::json!({
        "production": [{"name": "cpu-alerts", "rules": []}]
    });

    Mock::given(method("GET"))
        .and(path("/prometheus/config/v1/rules/production"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = GetRulesByNamespace::new(mimir, "production")
        .execute(&ctx())
        .await
        .unwrap();
    assert_eq!(result["production"][0]["name"], "cpu-alerts");
}

#[tokio::test]
async fn get_rule_group_returns_json() {
    let server = MockServer::start().await;
    let body = serde_json::json!({
        "name": "cpu-alerts",
        "interval": "1m",
        "rules": [{"alert": "HighCPU"}]
    });

    Mock::given(method("GET"))
        .and(path("/prometheus/config/v1/rules/production/cpu-alerts"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = GetRuleGroup::new(mimir, "production", "cpu-alerts")
        .execute(&ctx())
        .await
        .unwrap();
    assert_eq!(result["name"], "cpu-alerts");
}

#[tokio::test]
async fn delete_rule_namespace_sends_delete() {
    let server = MockServer::start().await;

    Mock::given(method("DELETE"))
        .and(path("/prometheus/config/v1/rules/staging"))
        .respond_with(ResponseTemplate::new(202))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = DeleteRuleNamespace::new(mimir, "staging")
        .execute(&ctx())
        .await
        .unwrap();
    assert_eq!(result["status"], "success");
}

#[tokio::test]
async fn get_rules_with_http_error() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/prometheus/api/v1/rules"))
        .respond_with(ResponseTemplate::new(500).set_body_string("internal error"))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let err = GetRules::new(mimir).execute(&ctx()).await.unwrap_err();
    match err {
        ironflow_core::error::OperationError::Http { status, message } => {
            assert_eq!(status, Some(500));
            assert_eq!(message, "internal error");
        }
        other => panic!("expected Http error, got: {other}"),
    }
}
