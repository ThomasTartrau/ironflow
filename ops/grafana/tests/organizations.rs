use std::sync::Arc;

use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_grafana::GrafanaClient;
use ironflow_ops_grafana::organizations::{
    OrgAddCurrentUser, OrgCreate, OrgDelete, OrgGet, OrgGetCurrent, OrgGetCurrentUsers, OrgList,
    OrgRemoveCurrentUser, OrgUpdateCurrent, OrgUpdateCurrentUser,
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
async fn org_get_current() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/org"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": 1, "name": "Main"})))
        .expect(1)
        .mount(&server)
        .await;

    let op = OrgGetCurrent::new(&client);
    assert_eq!(op.kind(), "grafana");
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["name"], "Main");
}

#[tokio::test]
async fn org_get_current_run_typed() {
    let (server, client, _ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/org"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": 2, "name": "Dev"})))
        .mount(&server)
        .await;

    let op = OrgGetCurrent::new(&client);
    let output = op.run().await.unwrap();
    assert_eq!(output.id, Some(2));
    assert_eq!(output.name.as_deref(), Some("Dev"));
}

#[tokio::test]
async fn org_update_current() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("PUT"))
        .and(path("/api/org"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"message": "updated"})))
        .expect(1)
        .mount(&server)
        .await;

    let op = OrgUpdateCurrent::new(&client, json!({"name": "Renamed"}));
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["message"], "updated");
}

#[tokio::test]
async fn org_get_current_users() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/org/users"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"orgId": 1, "userId": 10, "login": "admin", "role": "Admin"}
        ])))
        .expect(1)
        .mount(&server)
        .await;

    let op = OrgGetCurrentUsers::new(&client);
    let result = op.execute(&ctx).await.unwrap();
    assert!(result.is_array());
    assert_eq!(result[0]["login"], "admin");
}

#[tokio::test]
async fn org_add_current_user() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("POST"))
        .and(path("/api/org/users"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"message": "added"})))
        .expect(1)
        .mount(&server)
        .await;

    let op = OrgAddCurrentUser::new(
        &client,
        json!({"loginOrEmail": "user@ex.com", "role": "Viewer"}),
    );
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["message"], "added");
}

#[tokio::test]
async fn org_update_current_user() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("PATCH"))
        .and(path("/api/org/users/10"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"message": "updated"})))
        .expect(1)
        .mount(&server)
        .await;

    let op = OrgUpdateCurrentUser::new(&client, 10, json!({"role": "Editor"}));
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["message"], "updated");
}

#[tokio::test]
async fn org_remove_current_user() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("DELETE"))
        .and(path("/api/org/users/10"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"message": "removed"})))
        .expect(1)
        .mount(&server)
        .await;

    let op = OrgRemoveCurrentUser::new(&client, 10);
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["message"], "removed");
}

#[tokio::test]
async fn org_list() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/orgs"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id": 1, "name": "Main"}, {"id": 2, "name": "Dev"}
        ])))
        .expect(1)
        .mount(&server)
        .await;

    let op = OrgList::new(&client);
    let result = op.execute(&ctx).await.unwrap();
    assert!(result.is_array());
    assert_eq!(result.as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn org_get() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/orgs/5"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": 5, "name": "Prod"})))
        .expect(1)
        .mount(&server)
        .await;

    let op = OrgGet::new(&client, 5);
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["id"], 5);
}

#[tokio::test]
async fn org_create() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("POST"))
        .and(path("/api/orgs"))
        .and(bearer_token("test-token"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"orgId": 3, "message": "created"})),
        )
        .expect(1)
        .mount(&server)
        .await;

    let op = OrgCreate::new(&client, "New Org");
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["orgId"], 3);
}

#[tokio::test]
async fn org_delete() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("DELETE"))
        .and(path("/api/orgs/3"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"message": "deleted"})))
        .expect(1)
        .mount(&server)
        .await;

    let op = OrgDelete::new(&client, 3);
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["message"], "deleted");
}
