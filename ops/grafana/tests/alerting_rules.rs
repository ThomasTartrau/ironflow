use std::sync::Arc;

use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_grafana::GrafanaClient;
use ironflow_ops_grafana::alerting::{
    AlertRuleCreate, AlertRuleDelete, AlertRuleGet, AlertRuleGroupGet, AlertRuleGroupUpdate,
    AlertRuleList, AlertRuleUpdate,
};
use serde_json::json;
use wiremock::matchers::{bearer_token, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn setup() -> (MockServer, GrafanaClient, OperationContext) {
    let server = MockServer::start().await;
    let client = GrafanaClient::new("test-token", &server.uri()).unwrap();
    let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
    (server, client, ctx)
}

#[tokio::test]
async fn alert_rule_list() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/provisioning/alert-rules"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"uid": "r1", "title": "CPU High"}
        ])))
        .expect(1)
        .mount(&server)
        .await;

    let op = AlertRuleList::new(&client);
    assert_eq!(op.kind(), "grafana");
    let result = op.execute(&ctx).await.unwrap();
    assert!(result.is_array());
    assert_eq!(result[0]["uid"], "r1");
}

#[tokio::test]
async fn alert_rule_list_run_typed() {
    let (server, client, _ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/provisioning/alert-rules"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"uid": "r2", "title": "Mem High", "folderUid": "f1", "ruleGroup": "infra"}
        ])))
        .mount(&server)
        .await;

    let op = AlertRuleList::new(&client);
    let rules = op.run().await.unwrap();
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].uid.as_deref(), Some("r2"));
    assert_eq!(rules[0].folder_uid.as_deref(), Some("f1"));
}

#[tokio::test]
async fn alert_rule_get() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/provisioning/alert-rules/rule-abc"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "uid": "rule-abc", "title": "Disk Full"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let op = AlertRuleGet::new(&client, "rule-abc");
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["uid"], "rule-abc");
}

#[tokio::test]
async fn alert_rule_get_run_typed() {
    let (server, client, _ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/provisioning/alert-rules/typed-rule"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "uid": "typed-rule", "title": "Typed Alert", "folderUid": "f3", "ruleGroup": "sre"
        })))
        .mount(&server)
        .await;

    let op = AlertRuleGet::new(&client, "typed-rule");
    let output = op.run().await.unwrap();
    assert_eq!(output.uid.as_deref(), Some("typed-rule"));
    assert_eq!(output.title.as_deref(), Some("Typed Alert"));
    assert_eq!(output.folder_uid.as_deref(), Some("f3"));
}

#[tokio::test]
async fn alert_rule_create() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/provisioning/alert-rules"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "uid": "new-rule", "title": "New Alert"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let body = json!({"title": "New Alert", "condition": "A"});
    let op = AlertRuleCreate::new(&client, body);
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["uid"], "new-rule");
}

#[tokio::test]
async fn alert_rule_create_run_typed() {
    let (server, client, _ctx) = setup().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/provisioning/alert-rules"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "uid": "cr-rule", "title": "Created Rule", "ruleGroup": "infra"
        })))
        .mount(&server)
        .await;

    let op = AlertRuleCreate::new(&client, json!({"title": "Created Rule"}));
    let output = op.run().await.unwrap();
    assert_eq!(output.uid.as_deref(), Some("cr-rule"));
    assert_eq!(output.rule_group.as_deref(), Some("infra"));
}

#[tokio::test]
async fn alert_rule_update() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("PUT"))
        .and(path("/api/v1/provisioning/alert-rules/upd-rule"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "uid": "upd-rule", "title": "Updated"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let op = AlertRuleUpdate::new(&client, "upd-rule", json!({"title": "Updated"}));
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["title"], "Updated");
}

#[tokio::test]
async fn alert_rule_delete() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("DELETE"))
        .and(path("/api/v1/provisioning/alert-rules/del-rule"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"message": "deleted"})))
        .expect(1)
        .mount(&server)
        .await;

    let op = AlertRuleDelete::new(&client, "del-rule");
    let result = op.execute(&ctx).await.unwrap();
    assert!(result.is_object());
}

#[tokio::test]
async fn alert_rule_group_get() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/provisioning/folder/f1/rule-groups/infra"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "name": "infra", "interval": "1m", "rules": [{"uid": "r1"}]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let op = AlertRuleGroupGet::new(&client, "f1", "infra");
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["name"], "infra");
}

#[tokio::test]
async fn alert_rule_group_get_run_typed() {
    let (server, client, _ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/provisioning/folder/f2/rule-groups/apps"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "name": "apps", "interval": "5m", "rules": []
        })))
        .mount(&server)
        .await;

    let op = AlertRuleGroupGet::new(&client, "f2", "apps");
    let output = op.run().await.unwrap();
    assert_eq!(output.name.as_deref(), Some("apps"));
    assert_eq!(output.interval.as_deref(), Some("5m"));
}

#[tokio::test]
async fn alert_rule_group_update() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("PUT"))
        .and(path("/api/v1/provisioning/folder/f1/rule-groups/infra"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "name": "infra", "interval": "2m"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let op = AlertRuleGroupUpdate::new(&client, "f1", "infra", json!({"interval": "2m"}));
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["interval"], "2m");
}
