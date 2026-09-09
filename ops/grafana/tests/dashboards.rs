use std::sync::Arc;

use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_grafana::GrafanaClient;
use ironflow_ops_grafana::dashboards::{
    DashboardCreate, DashboardDelete, DashboardGet, DashboardGetPermissions, DashboardGetVersion,
    DashboardGetVersions, DashboardRestoreVersion, DashboardSearch, DashboardUpdate,
    DashboardUpdatePermissions,
};
use serde_json::json;
use wiremock::matchers::{bearer_token, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn setup() -> (MockServer, GrafanaClient, OperationContext) {
    let server = MockServer::start().await;
    let client = GrafanaClient::new("test-token", &server.uri()).unwrap();
    let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
    (server, client, ctx)
}

#[tokio::test]
async fn dashboard_get_builds_correct_url() {
    let (server, client, ctx) = setup().await;

    Mock::given(method("GET"))
        .and(path("/api/dashboards/uid/test-uid"))
        .and(bearer_token("test-token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"meta": {"slug": "test"}, "dashboard": {"id": 1}})),
        )
        .expect(1)
        .mount(&server)
        .await;

    let op = DashboardGet::new(&client, "test-uid");
    let result = op.execute(&ctx).await.unwrap();
    assert!(result.is_object());
    assert!(result["dashboard"]["id"] == 1);
}

#[tokio::test]
async fn dashboard_get_run_returns_typed() {
    let (server, client, _ctx) = setup().await;

    Mock::given(method("GET"))
        .and(path("/api/dashboards/uid/typed-uid"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"meta": {"slug": "typed"}, "dashboard": {"title": "My Dashboard"}}),
        ))
        .expect(1)
        .mount(&server)
        .await;

    let op = DashboardGet::new(&client, "typed-uid");
    let output = op.run().await.unwrap();
    assert!(output.dashboard.is_some());
}

#[tokio::test]
async fn dashboard_create_sends_correct_body() {
    let (server, client, ctx) = setup().await;

    Mock::given(method("POST"))
        .and(path("/api/dashboards/db"))
        .and(bearer_token("test-token"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(
                json!({"id": 1, "uid": "new-uid", "status": "success", "version": 1}),
            ),
        )
        .expect(1)
        .mount(&server)
        .await;

    let body = json!({"dashboard": {"title": "New"}, "overwrite": false});
    let op = DashboardCreate::new(&client, body.clone());

    assert_eq!(op.kind(), "grafana");
    assert_eq!(op.input().unwrap(), body);

    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["uid"], "new-uid");
}

#[tokio::test]
async fn dashboard_create_run_returns_typed() {
    let (server, client, _ctx) = setup().await;

    Mock::given(method("POST"))
        .and(path("/api/dashboards/db"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"id": 42, "uid": "abc", "status": "success", "version": 1})),
        )
        .mount(&server)
        .await;

    let op = DashboardCreate::new(&client, json!({"dashboard": {"title": "Test"}}));
    let output = op.run().await.unwrap();
    assert_eq!(output.id, Some(42));
    assert_eq!(output.uid.as_deref(), Some("abc"));
}

#[tokio::test]
async fn dashboard_update_uses_same_endpoint() {
    let (server, client, ctx) = setup().await;

    Mock::given(method("POST"))
        .and(path("/api/dashboards/db"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"id": 1, "uid": "upd", "status": "success", "version": 2})),
        )
        .expect(1)
        .mount(&server)
        .await;

    let body = json!({"dashboard": {"id": 1, "title": "Updated"}, "overwrite": true});
    let op = DashboardUpdate::new(&client, body);
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["version"], 2);
}

#[tokio::test]
async fn dashboard_delete_sends_delete_request() {
    let (server, client, ctx) = setup().await;

    Mock::given(method("DELETE"))
        .and(path("/api/dashboards/uid/del-uid"))
        .and(bearer_token("test-token"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"message": "Dashboard deleted"})),
        )
        .expect(1)
        .mount(&server)
        .await;

    let op = DashboardDelete::new(&client, "del-uid");
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["message"], "Dashboard deleted");
}

#[tokio::test]
async fn dashboard_search_with_query() {
    let (server, client, ctx) = setup().await;

    Mock::given(method("GET"))
        .and(path("/api/search"))
        .and(query_param("type", "dash-db"))
        .and(query_param("query", "prod"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id": 1, "uid": "a", "title": "Production", "type": "dash-db"}
        ])))
        .expect(1)
        .mount(&server)
        .await;

    let op = DashboardSearch::new(&client, Some("prod"), None);
    let result = op.execute(&ctx).await.unwrap();
    assert!(result.is_array());
    assert_eq!(result[0]["title"], "Production");
}

#[tokio::test]
async fn dashboard_search_run_returns_typed() {
    let (server, client, _ctx) = setup().await;

    Mock::given(method("GET"))
        .and(path("/api/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id": 1, "uid": "x", "title": "Test", "type": "dash-db", "tags": ["tag1"]}
        ])))
        .mount(&server)
        .await;

    let op = DashboardSearch::new(&client, None, None);
    let hits = op.run().await.unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].title.as_deref(), Some("Test"));
}

#[tokio::test]
async fn dashboard_get_versions() {
    let (server, client, ctx) = setup().await;

    Mock::given(method("GET"))
        .and(path("/api/dashboards/id/42/versions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id": 1, "dashboardId": 42, "message": "initial"}
        ])))
        .expect(1)
        .mount(&server)
        .await;

    let op = DashboardGetVersions::new(&client, 42);
    let result = op.execute(&ctx).await.unwrap();
    assert!(result.is_array());
}

#[tokio::test]
async fn dashboard_get_version() {
    let (server, client, ctx) = setup().await;

    Mock::given(method("GET"))
        .and(path("/api/dashboards/id/42/versions/3"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": 3, "data": {}})))
        .expect(1)
        .mount(&server)
        .await;

    let op = DashboardGetVersion::new(&client, 42, 3);
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["id"], 3);
}

#[tokio::test]
async fn dashboard_restore_version() {
    let (server, client, ctx) = setup().await;

    Mock::given(method("POST"))
        .and(path("/api/dashboards/id/42/restore"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "success"})))
        .expect(1)
        .mount(&server)
        .await;

    let op = DashboardRestoreVersion::new(&client, 42, 2);
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["status"], "success");
}

#[tokio::test]
async fn dashboard_get_permissions() {
    let (server, client, ctx) = setup().await;

    Mock::given(method("GET"))
        .and(path("/api/dashboards/uid/perm-uid/permissions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"dashboardId": 1, "role": "Viewer", "permission": 1}
        ])))
        .expect(1)
        .mount(&server)
        .await;

    let op = DashboardGetPermissions::new(&client, "perm-uid");
    let result = op.execute(&ctx).await.unwrap();
    assert!(result.is_array());
}

#[tokio::test]
async fn dashboard_update_permissions() {
    let (server, client, ctx) = setup().await;

    Mock::given(method("POST"))
        .and(path("/api/dashboards/uid/perm-uid/permissions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"message": "ok"})))
        .expect(1)
        .mount(&server)
        .await;

    let items = json!({"items": [{"role": "Viewer", "permission": 1}]});
    let op = DashboardUpdatePermissions::new(&client, "perm-uid", items);
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["message"], "ok");
}
