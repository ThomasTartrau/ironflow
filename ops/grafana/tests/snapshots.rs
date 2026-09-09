use std::sync::Arc;

use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_grafana::GrafanaClient;
use ironflow_ops_grafana::snapshots::{
    SnapshotCreate, SnapshotDeleteByKey, SnapshotGetByKey, SnapshotList,
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
async fn snapshot_create() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("POST"))
        .and(path("/api/snapshots"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "key": "snap-key", "deleteKey": "del-key",
            "url": "/dashboard/snapshot/snap-key",
            "deleteUrl": "/api/snapshots-delete/del-key"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let op = SnapshotCreate::new(&client, json!({"dashboard": {"title": "Snap"}}));
    assert_eq!(op.kind(), "grafana");
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["key"], "snap-key");
}

#[tokio::test]
async fn snapshot_create_run_typed() {
    let (server, client, _ctx) = setup().await;
    Mock::given(method("POST"))
        .and(path("/api/snapshots"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "key": "k1", "deleteKey": "dk1", "url": "/u", "deleteUrl": "/du"
        })))
        .mount(&server)
        .await;

    let op = SnapshotCreate::new(&client, json!({"dashboard": {}}));
    let output = op.run().await.unwrap();
    assert_eq!(output.key.as_deref(), Some("k1"));
    assert_eq!(output.delete_key.as_deref(), Some("dk1"));
}

#[tokio::test]
async fn snapshot_list() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/dashboard/snapshots"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"key": "s1", "name": "Snap1", "external": false}
        ])))
        .expect(1)
        .mount(&server)
        .await;

    let op = SnapshotList::new(&client);
    let result = op.execute(&ctx).await.unwrap();
    assert!(result.is_array());
    assert_eq!(result[0]["key"], "s1");
}

#[tokio::test]
async fn snapshot_list_run_typed() {
    let (server, client, _ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/dashboard/snapshots"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"key": "s2", "name": "Snap2", "external": true, "expires": "2030-01-01"}
        ])))
        .mount(&server)
        .await;

    let op = SnapshotList::new(&client);
    let items = op.run().await.unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].external, Some(true));
}

#[tokio::test]
async fn snapshot_get_by_key() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/snapshots/snap-key"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "dashboard": {"title": "Snapshot Dashboard"}
        })))
        .expect(1)
        .mount(&server)
        .await;

    let op = SnapshotGetByKey::new(&client, "snap-key");
    let result = op.execute(&ctx).await.unwrap();
    assert!(result["dashboard"].is_object());
}

#[tokio::test]
async fn snapshot_delete_by_key() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("DELETE"))
        .and(path("/api/snapshots/snap-key"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"message": "deleted"})))
        .expect(1)
        .mount(&server)
        .await;

    let op = SnapshotDeleteByKey::new(&client, "snap-key");
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["message"], "deleted");
}
