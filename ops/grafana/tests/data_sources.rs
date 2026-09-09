use std::sync::Arc;

use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_grafana::GrafanaClient;
use ironflow_ops_grafana::data_sources::{
    DataSourceCreate, DataSourceDelete, DataSourceGetById, DataSourceGetByName, DataSourceGetByUid,
    DataSourceList, DataSourceQuery, DataSourceUpdate,
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
async fn data_source_create() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("POST"))
        .and(path("/api/datasources"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 1, "uid": "ds1", "name": "Prometheus", "type": "prometheus"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let body = json!({"name": "Prometheus", "type": "prometheus", "url": "http://prom:9090"});
    let op = DataSourceCreate::new(&client, body);
    assert_eq!(op.kind(), "grafana");
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["name"], "Prometheus");
}

#[tokio::test]
async fn data_source_list() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/datasources"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id": 1, "uid": "ds1", "name": "Prom", "type": "prometheus"}
        ])))
        .expect(1)
        .mount(&server)
        .await;

    let op = DataSourceList::new(&client);
    let result = op.execute(&ctx).await.unwrap();
    assert!(result.is_array());
    assert_eq!(result[0]["uid"], "ds1");
}

#[tokio::test]
async fn data_source_list_run_typed() {
    let (server, client, _ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/datasources"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id": 2, "uid": "ds2", "name": "Loki", "type": "loki", "url": "http://loki:3100"}
        ])))
        .mount(&server)
        .await;

    let op = DataSourceList::new(&client);
    let sources = op.run().await.unwrap();
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].name.as_deref(), Some("Loki"));
}

#[tokio::test]
async fn data_source_get_by_id() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/datasources/42"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 42, "uid": "ds42", "name": "InfluxDB", "type": "influxdb"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let op = DataSourceGetById::new(&client, 42);
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["id"], 42);
}

#[tokio::test]
async fn data_source_get_by_uid() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/datasources/uid/ds-uid"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 5, "uid": "ds-uid", "name": "Tempo"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let op = DataSourceGetByUid::new(&client, "ds-uid");
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["uid"], "ds-uid");
}

#[tokio::test]
async fn data_source_get_by_name() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/datasources/name/Prometheus"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 1, "name": "Prometheus", "type": "prometheus"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let op = DataSourceGetByName::new(&client, "Prometheus");
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["name"], "Prometheus");
}

#[tokio::test]
async fn data_source_update() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("PUT"))
        .and(path("/api/datasources/1"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 1, "name": "Updated DS"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let op = DataSourceUpdate::new(&client, 1, json!({"name": "Updated DS"}));
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["name"], "Updated DS");
}

#[tokio::test]
async fn data_source_delete() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("DELETE"))
        .and(path("/api/datasources/99"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"message": "deleted"})))
        .expect(1)
        .mount(&server)
        .await;

    let op = DataSourceDelete::new(&client, 99);
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["message"], "deleted");
}

#[tokio::test]
async fn data_source_query() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("POST"))
        .and(path("/api/ds/query"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": {"A": {"frames": []}}
        })))
        .expect(1)
        .mount(&server)
        .await;

    let body = json!({"queries": [{"datasourceId": 1, "expr": "up"}]});
    let op = DataSourceQuery::new(&client, body);
    let result = op.execute(&ctx).await.unwrap();
    assert!(result["results"].is_object());
}
