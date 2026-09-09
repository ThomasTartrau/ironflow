use std::sync::Arc;

use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_grafana::GrafanaClient;
use ironflow_ops_grafana::playlists::{
    PlaylistCreate, PlaylistDelete, PlaylistGet, PlaylistList, PlaylistUpdate,
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
async fn playlist_list() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/playlists"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id": 1, "uid": "pl1", "name": "Rotation", "interval": "5m"}
        ])))
        .expect(1)
        .mount(&server)
        .await;

    let op = PlaylistList::new(&client);
    assert_eq!(op.kind(), "grafana");
    let result = op.execute(&ctx).await.unwrap();
    assert!(result.is_array());
    assert_eq!(result[0]["name"], "Rotation");
}

#[tokio::test]
async fn playlist_list_run_typed() {
    let (server, client, _ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/playlists"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id": 2, "uid": "pl2", "name": "TV", "interval": "10s"}
        ])))
        .mount(&server)
        .await;

    let op = PlaylistList::new(&client);
    let items = op.run().await.unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].interval.as_deref(), Some("10s"));
}

#[tokio::test]
async fn playlist_get() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/playlists/pl1"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 1, "uid": "pl1", "name": "Rotation", "interval": "5m"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let op = PlaylistGet::new(&client, "pl1");
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["uid"], "pl1");
}

#[tokio::test]
async fn playlist_get_run_typed() {
    let (server, client, _ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/playlists/typed-pl"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 5, "uid": "typed-pl", "name": "Typed", "interval": "30s"
        })))
        .mount(&server)
        .await;

    let op = PlaylistGet::new(&client, "typed-pl");
    let output = op.run().await.unwrap();
    assert_eq!(output.name.as_deref(), Some("Typed"));
    assert_eq!(output.interval.as_deref(), Some("30s"));
}

#[tokio::test]
async fn playlist_create() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("POST"))
        .and(path("/api/playlists"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 3, "uid": "new-pl", "name": "New", "interval": "1m"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let op = PlaylistCreate::new(
        &client,
        json!({"name": "New", "interval": "1m", "items": []}),
    );
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["uid"], "new-pl");
}

#[tokio::test]
async fn playlist_create_run_typed() {
    let (server, client, _ctx) = setup().await;
    Mock::given(method("POST"))
        .and(path("/api/playlists"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 10, "uid": "cr-pl", "name": "Created", "interval": "2m"
        })))
        .mount(&server)
        .await;

    let op = PlaylistCreate::new(&client, json!({"name": "Created", "interval": "2m"}));
    let output = op.run().await.unwrap();
    assert_eq!(output.uid.as_deref(), Some("cr-pl"));
    assert_eq!(output.name.as_deref(), Some("Created"));
}

#[tokio::test]
async fn playlist_update() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("PUT"))
        .and(path("/api/playlists/pl1"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 1, "uid": "pl1", "name": "Updated", "interval": "2m"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let op = PlaylistUpdate::new(&client, "pl1", json!({"name": "Updated", "interval": "2m"}));
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["name"], "Updated");
}

#[tokio::test]
async fn playlist_delete() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("DELETE"))
        .and(path("/api/playlists/pl1"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"message": "deleted"})))
        .expect(1)
        .mount(&server)
        .await;

    let op = PlaylistDelete::new(&client, "pl1");
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["message"], "deleted");
}
