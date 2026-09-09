use std::sync::Arc;

use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_grafana::GrafanaClient;
use ironflow_ops_grafana::other::{CreateShortUrl, GetFrontendSettings, RenewAuth};
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
async fn create_short_url() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("POST"))
        .and(path("/api/short-urls"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "uid": "short1", "url": "/goto/short1"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let op = CreateShortUrl::new(&client, json!({"path": "/d/abc/my-dashboard"}));
    assert_eq!(op.kind(), "grafana");
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["uid"], "short1");
}

#[tokio::test]
async fn create_short_url_run_typed() {
    let (server, client, _ctx) = setup().await;
    Mock::given(method("POST"))
        .and(path("/api/short-urls"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "uid": "s2", "url": "/goto/s2"
        })))
        .mount(&server)
        .await;

    let op = CreateShortUrl::new(&client, json!({"path": "/test"}));
    let output = op.run().await.unwrap();
    assert_eq!(output.uid.as_deref(), Some("s2"));
}

#[tokio::test]
async fn get_frontend_settings() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/frontend/settings"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "defaultDatasource": "Prometheus",
            "alertingEnabled": true
        })))
        .expect(1)
        .mount(&server)
        .await;

    let op = GetFrontendSettings::new(&client);
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["defaultDatasource"], "Prometheus");
}

#[tokio::test]
async fn renew_auth() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/auth/renew"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "token": "new-token", "expiry": "2030-01-01"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let op = RenewAuth::new(&client);
    let result = op.execute(&ctx).await.unwrap();
    assert!(result.is_object());
}
