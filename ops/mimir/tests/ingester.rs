use std::sync::Arc;

use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_mimir::MimirClient;
use ironflow_ops_mimir::ingester::{
    CancelPartitionDownscale, CancelShutdown, Flush, GetIngesterRing, GetIngesterTenants,
    PreparePartitionDownscale, PrepareShutdown, Shutdown,
};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn ctx() -> OperationContext {
    OperationContext::new(Arc::new(NoopSecretResolver))
}

#[tokio::test]
async fn flush_sends_post_and_returns_success() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/ingester/flush"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = Flush::new(mimir).execute(&ctx()).await.unwrap();
    assert_eq!(result["status"], "success");
}

#[tokio::test]
async fn prepare_shutdown_sends_post() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/ingester/prepare-shutdown"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = PrepareShutdown::new(mimir).execute(&ctx()).await.unwrap();
    assert_eq!(result["status"], "success");
}

#[tokio::test]
async fn cancel_shutdown_sends_delete() {
    let server = MockServer::start().await;

    Mock::given(method("DELETE"))
        .and(path("/ingester/prepare-shutdown"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = CancelShutdown::new(mimir).execute(&ctx()).await.unwrap();
    assert_eq!(result["status"], "success");
}

#[tokio::test]
async fn shutdown_sends_post() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/ingester/shutdown"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = Shutdown::new(mimir).execute(&ctx()).await.unwrap();
    assert_eq!(result["status"], "success");
}

#[tokio::test]
async fn prepare_partition_downscale_sends_post() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/ingester/prepare-partition-downscale"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = PreparePartitionDownscale::new(mimir)
        .execute(&ctx())
        .await
        .unwrap();
    assert_eq!(result["status"], "success");
}

#[tokio::test]
async fn cancel_partition_downscale_sends_delete() {
    let server = MockServer::start().await;

    Mock::given(method("DELETE"))
        .and(path("/ingester/prepare-partition-downscale"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = CancelPartitionDownscale::new(mimir)
        .execute(&ctx())
        .await
        .unwrap();
    assert_eq!(result["status"], "success");
}

#[tokio::test]
async fn get_ingester_ring_returns_text_wrapped_in_json() {
    let server = MockServer::start().await;
    let ring_text = "Ring: ACTIVE, tokens: 512";

    Mock::given(method("GET"))
        .and(path("/ingester/ring"))
        .respond_with(ResponseTemplate::new(200).set_body_string(ring_text))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = GetIngesterRing::new(mimir).execute(&ctx()).await.unwrap();
    assert!(result["ring"].as_str().unwrap().contains("ACTIVE"));
}

#[tokio::test]
async fn get_ingester_tenants_returns_json() {
    let server = MockServer::start().await;
    let body = serde_json::json!(["tenant-1", "tenant-2"]);

    Mock::given(method("GET"))
        .and(path("/ingester/tenants"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = GetIngesterTenants::new(mimir)
        .execute(&ctx())
        .await
        .unwrap();
    assert_eq!(result[0], "tenant-1");
}

#[tokio::test]
async fn flush_with_http_error() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/ingester/flush"))
        .respond_with(ResponseTemplate::new(500).set_body_string("flush failed"))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let err = Flush::new(mimir).execute(&ctx()).await.unwrap_err();
    match err {
        ironflow_core::error::OperationError::Http { status, message } => {
            assert_eq!(status, Some(500));
            assert_eq!(message, "flush failed");
        }
        other => panic!("expected Http error, got: {other}"),
    }
}
