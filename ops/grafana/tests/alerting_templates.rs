use std::sync::Arc;

use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_grafana::GrafanaClient;
use ironflow_ops_grafana::alerting::{
    TemplateCreate, TemplateDelete, TemplateList, TemplateUpdate,
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
async fn template_list() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/provisioning/templates"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"name": "default", "template": "{{ .Alerts }}"}
        ])))
        .expect(1)
        .mount(&server)
        .await;

    let op = TemplateList::new(&client);
    let result = op.execute(&ctx).await.unwrap();
    assert!(result.is_array());
    assert_eq!(result[0]["name"], "default");
}

#[tokio::test]
async fn template_list_run_typed() {
    let (server, client, _ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/provisioning/templates"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"name": "custom", "template": "custom body"}
        ])))
        .mount(&server)
        .await;

    let op = TemplateList::new(&client);
    let templates = op.run().await.unwrap();
    assert_eq!(templates.len(), 1);
    assert_eq!(templates[0].name.as_deref(), Some("custom"));
}

#[tokio::test]
async fn template_create() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("PUT"))
        .and(path("/api/v1/provisioning/templates/new-tpl"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(202).set_body_json(json!({
            "name": "new-tpl", "template": "body"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let op = TemplateCreate::new(&client, "new-tpl", json!({"template": "body"}));
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["name"], "new-tpl");
}

#[tokio::test]
async fn template_update() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("PUT"))
        .and(path("/api/v1/provisioning/templates/existing"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(202).set_body_json(json!({
            "name": "existing", "template": "updated"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let op = TemplateUpdate::new(&client, "existing", json!({"template": "updated"}));
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["template"], "updated");
}

#[tokio::test]
async fn template_delete() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("DELETE"))
        .and(path("/api/v1/provisioning/templates/old"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"message": "deleted"})))
        .expect(1)
        .mount(&server)
        .await;

    let op = TemplateDelete::new(&client, "old");
    let result = op.execute(&ctx).await.unwrap();
    assert!(result.is_object());
}
