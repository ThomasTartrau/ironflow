use std::sync::Arc;

use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_loki::LokiClient;
use ironflow_ops_loki::rules::{CreateRuleGroup, DeleteRuleGroup, GetAlerts, GetRules};
use serde_json::json;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn ctx() -> OperationContext {
    OperationContext::new(Arc::new(NoopSecretResolver))
}

#[tokio::test]
async fn get_rules_returns_alerting_rules() {
    let server = MockServer::start().await;
    let body = json!({
        "status": "success",
        "data": {
            "groups": [{
                "name": "test-group",
                "rules": [{
                    "name": "HighErrorRate",
                    "query": r#"sum(rate({job="app"} |= "error" [5m])) > 10"#,
                    "type": "alerting",
                    "health": "ok"
                }]
            }]
        }
    });

    Mock::given(method("GET"))
        .and(path("/loki/api/v1/rules"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let loki = LokiClient::new(&server.uri(), reqwest::Client::new());
    let op = GetRules::new(loki);
    let result = op.execute(&ctx()).await.unwrap();

    assert_eq!(result["status"], "success");
    let groups = result["data"]["groups"].as_array().unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0]["name"], "test-group");
    assert_eq!(groups[0]["rules"][0]["type"], "alerting");
}

#[tokio::test]
async fn create_rule_group_sends_yaml_body_with_content_type() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/loki/api/v1/rules/production"))
        .and(header("Content-Type", "application/yaml"))
        .respond_with(ResponseTemplate::new(202))
        .mount(&server)
        .await;

    let loki = LokiClient::new(&server.uri(), reqwest::Client::new());
    let yaml = "name: test-group\ninterval: 1m\nrules: []";
    let op = CreateRuleGroup::new(loki, "production", yaml);

    let result = op.execute(&ctx()).await.unwrap();
    assert_eq!(result["status"], "success");
}

#[tokio::test]
async fn delete_rule_group_sends_delete_to_correct_path() {
    let server = MockServer::start().await;

    Mock::given(method("DELETE"))
        .and(path("/loki/api/v1/rules/production/old-group"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let loki = LokiClient::new(&server.uri(), reqwest::Client::new());
    let op = DeleteRuleGroup::new(loki, "production", "old-group");

    let result = op.execute(&ctx()).await.unwrap();
    assert_eq!(result["status"], "success");
}

#[tokio::test]
async fn get_alerts_returns_active_alerts() {
    let server = MockServer::start().await;
    let body = json!({
        "status": "success",
        "data": {
            "alerts": [{
                "labels": {"alertname": "HighErrorRate"},
                "state": "firing"
            }]
        }
    });

    Mock::given(method("GET"))
        .and(path("/loki/api/v1/alerts"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let loki = LokiClient::new(&server.uri(), reqwest::Client::new());
    let op = GetAlerts::new(loki);

    let result = op.execute(&ctx()).await.unwrap();
    assert_eq!(result["data"]["alerts"][0]["state"], "firing");
}
