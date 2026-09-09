use std::sync::Arc;

use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_grafana::GrafanaClient;
use ironflow_ops_grafana::service_accounts::{
    ServiceAccountCreate, ServiceAccountCreateToken, ServiceAccountDelete,
    ServiceAccountDeleteToken, ServiceAccountGet, ServiceAccountList, ServiceAccountUpdate,
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
async fn sa_list() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/serviceaccounts/search"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "serviceAccounts": [{"id": 1, "name": "bot"}], "totalCount": 1
        })))
        .expect(1)
        .mount(&server)
        .await;

    let op = ServiceAccountList::new(&client);
    assert_eq!(op.kind(), "grafana");
    let result = op.execute(&ctx).await.unwrap();
    assert!(result["serviceAccounts"].is_array());
}

#[tokio::test]
async fn sa_get() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/serviceaccounts/5"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 5, "name": "deployer", "role": "Editor"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let op = ServiceAccountGet::new(&client, 5);
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["name"], "deployer");
}

#[tokio::test]
async fn sa_get_run_typed() {
    let (server, client, _ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/serviceaccounts/5"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 5, "name": "ci", "login": "sa-ci", "role": "Viewer", "isDisabled": false
        })))
        .mount(&server)
        .await;

    let op = ServiceAccountGet::new(&client, 5);
    let output = op.run().await.unwrap();
    assert_eq!(output.name.as_deref(), Some("ci"));
    assert_eq!(output.is_disabled, Some(false));
}

#[tokio::test]
async fn sa_create() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("POST"))
        .and(path("/api/serviceaccounts"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "id": 10, "name": "new-sa", "role": "Viewer"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let op = ServiceAccountCreate::new(&client, json!({"name": "new-sa", "role": "Viewer"}));
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["id"], 10);
}

#[tokio::test]
async fn sa_update() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("PATCH"))
        .and(path("/api/serviceaccounts/10"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 10, "name": "updated-sa"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let op = ServiceAccountUpdate::new(&client, 10, json!({"name": "updated-sa"}));
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["name"], "updated-sa");
}

#[tokio::test]
async fn sa_delete() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("DELETE"))
        .and(path("/api/serviceaccounts/10"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"message": "deleted"})))
        .expect(1)
        .mount(&server)
        .await;

    let op = ServiceAccountDelete::new(&client, 10);
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["message"], "deleted");
}

#[tokio::test]
async fn sa_create_token() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("POST"))
        .and(path("/api/serviceaccounts/10/tokens"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 1, "name": "deploy-key", "key": "glsa_xxx"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let op = ServiceAccountCreateToken::new(
        &client,
        10,
        json!({"name": "deploy-key", "secondsToLive": 86400}),
    );
    let result = op.execute(&ctx).await.unwrap();
    // execute() redacts the token key -- it must not appear in step history.
    assert_eq!(result["key"], "[REDACTED]");
    // The un-redacted key is only available via the typed run() method.
}

#[tokio::test]
async fn sa_create_token_run_typed() {
    let (server, client, _ctx) = setup().await;
    Mock::given(method("POST"))
        .and(path("/api/serviceaccounts/10/tokens"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 2, "name": "ci-key", "key": "glsa_yyy"
        })))
        .mount(&server)
        .await;

    let op = ServiceAccountCreateToken::new(&client, 10, json!({"name": "ci-key"}));
    let output = op.run().await.unwrap();
    assert_eq!(output.key.as_deref(), Some("glsa_yyy"));
}

#[tokio::test]
async fn sa_delete_token() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("DELETE"))
        .and(path("/api/serviceaccounts/10/tokens/1"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"message": "deleted"})))
        .expect(1)
        .mount(&server)
        .await;

    let op = ServiceAccountDeleteToken::new(&client, 10, 1);
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["message"], "deleted");
}
