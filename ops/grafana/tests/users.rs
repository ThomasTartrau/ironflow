use std::sync::Arc;

use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_grafana::GrafanaClient;
use ironflow_ops_grafana::users::{UserGetById, UserList, UserSearch, UserUpdate};
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
async fn user_list() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/org/users"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id": 1, "login": "admin", "email": "admin@ex.com", "name": "Admin"}
        ])))
        .expect(1)
        .mount(&server)
        .await;

    let op = UserList::new(&client);
    assert_eq!(op.kind(), "grafana");
    let result = op.execute(&ctx).await.unwrap();
    assert!(result.is_array());
    assert_eq!(result[0]["login"], "admin");
}

#[tokio::test]
async fn user_list_run_typed() {
    let (server, client, _ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/org/users"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id": 2, "login": "dev", "isAdmin": false}
        ])))
        .mount(&server)
        .await;

    let op = UserList::new(&client);
    let users = op.run().await.unwrap();
    assert_eq!(users.len(), 1);
    assert_eq!(users[0].login.as_deref(), Some("dev"));
    assert_eq!(users[0].is_admin, Some(false));
}

#[tokio::test]
async fn user_get_by_id() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/users/42"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 42, "login": "alice", "email": "alice@ex.com", "name": "Alice"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let op = UserGetById::new(&client, 42);
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["id"], 42);
    assert_eq!(result["login"], "alice");
}

#[tokio::test]
async fn user_get_by_id_run_typed() {
    let (server, client, _ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/users/99"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 99, "login": "typed-user", "email": "typed@ex.com", "name": "Typed"
        })))
        .mount(&server)
        .await;

    let op = UserGetById::new(&client, 99);
    let output = op.run().await.unwrap();
    assert_eq!(output.login.as_deref(), Some("typed-user"));
    assert_eq!(output.email.as_deref(), Some("typed@ex.com"));
}

#[tokio::test]
async fn user_search() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("GET"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "users": [{"id": 3, "login": "bob"}],
            "totalCount": 1, "page": 1, "perPage": 50
        })))
        .expect(1)
        .mount(&server)
        .await;

    let op = UserSearch::new(&client, "bob");
    let result = op.execute(&ctx).await.unwrap();
    assert!(result["users"].is_array());
}

#[tokio::test]
async fn user_search_run_typed() {
    let (server, client, _ctx) = setup().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "users": [{"id": 4, "login": "eve"}],
            "totalCount": 1, "page": 1, "perPage": 50
        })))
        .mount(&server)
        .await;

    let op = UserSearch::new(&client, "eve");
    let output = op.run().await.unwrap();
    assert_eq!(output.total_count, Some(1));
}

#[tokio::test]
async fn user_update() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("PUT"))
        .and(path("/api/users/42"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"message": "updated"})))
        .expect(1)
        .mount(&server)
        .await;

    let op = UserUpdate::new(&client, 42, json!({"name": "Alice Updated"}));
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["message"], "updated");
}
