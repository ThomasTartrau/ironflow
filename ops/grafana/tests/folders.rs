use std::sync::Arc;

use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_grafana::GrafanaClient;
use ironflow_ops_grafana::folders::{
    FolderCreate, FolderDelete, FolderGet, FolderGetPermissions, FolderUpdate,
    FolderUpdatePermissions,
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
async fn folder_create() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("POST"))
        .and(path("/api/folders"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 1, "uid": "f-uid", "title": "New Folder"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let op = FolderCreate::new(&client, "New Folder", Some("f-uid"));
    assert_eq!(op.kind(), "grafana");
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["uid"], "f-uid");
}

#[tokio::test]
async fn folder_create_run_typed() {
    let (server, client, _ctx) = setup().await;
    Mock::given(method("POST"))
        .and(path("/api/folders"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 2, "uid": "auto", "title": "Auto"
        })))
        .mount(&server)
        .await;

    let op = FolderCreate::new(&client, "Auto", None);
    let output = op.run().await.unwrap();
    assert_eq!(output.uid.as_deref(), Some("auto"));
}

#[tokio::test]
async fn folder_get() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/folders/my-folder"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 3, "uid": "my-folder", "title": "My Folder"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let op = FolderGet::new(&client, "my-folder");
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["title"], "My Folder");
}

#[tokio::test]
async fn folder_get_run_typed() {
    let (server, client, _ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/folders/typed-uid"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 7, "uid": "typed-uid", "title": "Typed Folder", "url": "/folders/typed-uid"
        })))
        .mount(&server)
        .await;

    let op = FolderGet::new(&client, "typed-uid");
    let output = op.run().await.unwrap();
    assert_eq!(output.uid.as_deref(), Some("typed-uid"));
    assert_eq!(output.title.as_deref(), Some("Typed Folder"));
    assert_eq!(output.url.as_deref(), Some("/folders/typed-uid"));
}

#[tokio::test]
async fn folder_update() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("PUT"))
        .and(path("/api/folders/my-folder"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 3, "uid": "my-folder", "title": "Renamed"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let op = FolderUpdate::new(&client, "my-folder", "Renamed", 1);
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["title"], "Renamed");
}

#[tokio::test]
async fn folder_update_run_typed() {
    let (server, client, _ctx) = setup().await;
    Mock::given(method("PUT"))
        .and(path("/api/folders/upd-uid"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 3, "uid": "upd-uid", "title": "Updated Title"
        })))
        .mount(&server)
        .await;

    let op = FolderUpdate::new(&client, "upd-uid", "Updated Title", 2);
    let output = op.run().await.unwrap();
    assert_eq!(output.title.as_deref(), Some("Updated Title"));
}

#[tokio::test]
async fn folder_delete() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("DELETE"))
        .and(path("/api/folders/del-folder"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"message": "deleted"})))
        .expect(1)
        .mount(&server)
        .await;

    let op = FolderDelete::new(&client, "del-folder");
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["message"], "deleted");
}

#[tokio::test]
async fn folder_get_permissions() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/folders/perm-folder/permissions"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"folderId": 1, "role": "Editor", "permission": 2}
        ])))
        .expect(1)
        .mount(&server)
        .await;

    let op = FolderGetPermissions::new(&client, "perm-folder");
    let result = op.execute(&ctx).await.unwrap();
    assert!(result.is_array());
}

#[tokio::test]
async fn folder_get_permissions_run_typed() {
    let (server, client, _ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/folders/pf/permissions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"folderId": 5, "role": "Viewer", "permission": 1, "teamId": 10}
        ])))
        .mount(&server)
        .await;

    let op = FolderGetPermissions::new(&client, "pf");
    let perms = op.run().await.unwrap();
    assert_eq!(perms.len(), 1);
    assert_eq!(perms[0].role.as_deref(), Some("Viewer"));
}

#[tokio::test]
async fn folder_update_permissions() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("POST"))
        .and(path("/api/folders/perm-folder/permissions"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"message": "ok"})))
        .expect(1)
        .mount(&server)
        .await;

    let items = json!({"items": [{"role": "Admin", "permission": 4}]});
    let op = FolderUpdatePermissions::new(&client, "perm-folder", items);
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["message"], "ok");
}
