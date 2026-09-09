use std::sync::Arc;

use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_loki::LokiClient;
use ironflow_ops_loki::rings::{
    GetCompactorRing, GetDistributorRing, GetIndexGatewayRing, GetRulerRing,
};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn ctx() -> OperationContext {
    OperationContext::new(Arc::new(NoopSecretResolver))
}

#[tokio::test]
async fn get_distributor_ring_sends_get_to_correct_path() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/distributor/ring"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ring data"))
        .mount(&server)
        .await;

    let loki = LokiClient::new(&server.uri(), reqwest::Client::new());
    let op = GetDistributorRing::new(loki);

    let result = op.execute(&ctx()).await.unwrap();
    assert_eq!(result["ring"], "ring data");
}

#[tokio::test]
async fn get_index_gateway_ring_sends_get_to_correct_path() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/indexgateway/ring"))
        .respond_with(ResponseTemplate::new(200).set_body_string("igw ring"))
        .mount(&server)
        .await;

    let loki = LokiClient::new(&server.uri(), reqwest::Client::new());
    let op = GetIndexGatewayRing::new(loki);

    let result = op.execute(&ctx()).await.unwrap();
    assert_eq!(result["ring"], "igw ring");
}

#[tokio::test]
async fn get_ruler_ring_sends_get_to_correct_path() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/ruler/ring"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ruler ring"))
        .mount(&server)
        .await;

    let loki = LokiClient::new(&server.uri(), reqwest::Client::new());
    let op = GetRulerRing::new(loki);

    let result = op.execute(&ctx()).await.unwrap();
    assert_eq!(result["ring"], "ruler ring");
}

#[tokio::test]
async fn get_compactor_ring_sends_get_to_correct_path() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/compactor/ring"))
        .respond_with(ResponseTemplate::new(200).set_body_string("compactor ring"))
        .mount(&server)
        .await;

    let loki = LokiClient::new(&server.uri(), reqwest::Client::new());
    let op = GetCompactorRing::new(loki);

    let result = op.execute(&ctx()).await.unwrap();
    assert_eq!(result["ring"], "compactor ring");
}
