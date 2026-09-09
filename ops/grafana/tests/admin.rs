use std::sync::Arc;

use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_grafana::GrafanaClient;
use ironflow_ops_grafana::admin::{AdminGetHealth, AdminGetStats, AdminSetAlertsPause};
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
async fn admin_get_stats() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/admin/stats"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "users": 100, "orgs": 5, "dashboards": 50, "snapshots": 3,
            "tags": 20, "datasources": 8, "playlists": 2, "stars": 15,
            "alerts": 10, "activeUsers": 42
        })))
        .expect(1)
        .mount(&server)
        .await;

    let op = AdminGetStats::new(&client);
    assert_eq!(op.kind(), "grafana");
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["users"], 100);
}

#[tokio::test]
async fn admin_get_stats_run_typed() {
    let (server, client, _ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/admin/stats"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "users": 50, "orgs": 2, "dashboards": 20, "activeUsers": 10
        })))
        .mount(&server)
        .await;

    let op = AdminGetStats::new(&client);
    let output = op.run().await.unwrap();
    assert_eq!(output.users, Some(50));
    assert_eq!(output.active_users, Some(10));
}

#[tokio::test]
async fn admin_pause_all_alerts() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("POST"))
        .and(path("/api/admin/pause-all-alerts"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "state": "Paused", "message": "alerts paused"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let op = AdminSetAlertsPause::new(&client, true);
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["state"], "Paused");
}

#[tokio::test]
async fn admin_unpause_all_alerts() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("POST"))
        .and(path("/api/admin/pause-all-alerts"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "state": "Unpaused", "message": "alerts unpaused"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let op = AdminSetAlertsPause::new(&client, false);
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["state"], "Unpaused");
}

#[tokio::test]
async fn admin_get_health() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/health"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "commit": "abc123", "database": "ok", "version": "10.0.0"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let op = AdminGetHealth::new(&client);
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["database"], "ok");
}

#[tokio::test]
async fn admin_get_health_run_typed() {
    let (server, client, _ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "commit": "def456", "database": "ok", "version": "11.0.0"
        })))
        .mount(&server)
        .await;

    let op = AdminGetHealth::new(&client);
    let output = op.run().await.unwrap();
    assert_eq!(output.version.as_deref(), Some("11.0.0"));
    assert_eq!(output.database.as_deref(), Some("ok"));
}
