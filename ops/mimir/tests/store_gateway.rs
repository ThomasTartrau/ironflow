use std::sync::Arc;

use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_mimir::MimirClient;
use ironflow_ops_mimir::store_gateway::{GetRing, GetTenantBlocks, GetTenants, PrepareShutdown};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn ctx() -> OperationContext {
    OperationContext::new(Arc::new(NoopSecretResolver))
}

#[tokio::test]
async fn get_ring_returns_text_wrapped_in_json() {
    let server = MockServer::start().await;
    let ring_text = "Store-gateway ring: ACTIVE";

    Mock::given(method("GET"))
        .and(path("/store-gateway/ring"))
        .respond_with(ResponseTemplate::new(200).set_body_string(ring_text))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = GetRing::new(mimir).execute(&ctx()).await.unwrap();
    assert!(result["ring"].as_str().unwrap().contains("ACTIVE"));
}

#[tokio::test]
async fn get_tenants_returns_json() {
    let server = MockServer::start().await;
    let body = serde_json::json!(["tenant-a", "tenant-b"]);

    Mock::given(method("GET"))
        .and(path("/store-gateway/tenants"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = GetTenants::new(mimir).execute(&ctx()).await.unwrap();
    assert_eq!(result[0], "tenant-a");
}

#[tokio::test]
async fn get_tenant_blocks_returns_json() {
    let server = MockServer::start().await;
    let body = serde_json::json!({
        "blocks": [{"id": "01ABC", "minTime": 1000, "maxTime": 2000}]
    });

    Mock::given(method("GET"))
        .and(path("/store-gateway/tenants/tenant-1/blocks"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = GetTenantBlocks::new(mimir, "tenant-1")
        .execute(&ctx())
        .await
        .unwrap();
    assert_eq!(result["blocks"][0]["id"], "01ABC");
}

#[tokio::test]
async fn prepare_shutdown_sends_post() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/store-gateway/prepare-shutdown"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = PrepareShutdown::new(mimir).execute(&ctx()).await.unwrap();
    assert_eq!(result["status"], "success");
}

#[tokio::test]
async fn get_tenant_blocks_with_http_error() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/store-gateway/tenants/tenant-x/blocks"))
        .respond_with(ResponseTemplate::new(404).set_body_string("tenant not found"))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let err = GetTenantBlocks::new(mimir, "tenant-x")
        .execute(&ctx())
        .await
        .unwrap_err();
    match err {
        ironflow_core::error::OperationError::Http { status, message } => {
            assert_eq!(status, Some(404));
            assert_eq!(message, "tenant not found");
        }
        other => panic!("expected Http error, got: {other}"),
    }
}
