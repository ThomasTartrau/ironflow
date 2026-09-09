use std::sync::Arc;

use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_mimir::MimirClient;
use ironflow_ops_mimir::distributor::{
    GetDistributorRing, GetDistributorUserStats, GetHaTrackerStatus,
};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn ctx() -> OperationContext {
    OperationContext::new(Arc::new(NoopSecretResolver))
}

#[tokio::test]
async fn get_distributor_ring_returns_text_wrapped_in_json() {
    let server = MockServer::start().await;
    let ring_html = "<html><body>Ring status: ACTIVE</body></html>";

    Mock::given(method("GET"))
        .and(path("/distributor/ring"))
        .respond_with(ResponseTemplate::new(200).set_body_string(ring_html))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = GetDistributorRing::new(mimir)
        .execute(&ctx())
        .await
        .unwrap();
    assert!(result["ring"].as_str().unwrap().contains("ACTIVE"));
}

#[tokio::test]
async fn get_distributor_user_stats_returns_json() {
    let server = MockServer::start().await;
    let body = serde_json::json!([
        {"userID": "tenant-1", "ingestionRate": 100.0, "numSeries": 5000}
    ]);

    Mock::given(method("GET"))
        .and(path("/distributor/all_user_stats"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = GetDistributorUserStats::new(mimir)
        .execute(&ctx())
        .await
        .unwrap();
    assert_eq!(result[0]["userID"], "tenant-1");
}

#[tokio::test]
async fn get_ha_tracker_status_returns_json() {
    let server = MockServer::start().await;
    let body = serde_json::json!({
        "clusters": [{"cluster": "cluster-1", "replicas": 3}]
    });

    Mock::given(method("GET"))
        .and(path("/distributor/ha_tracker"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = GetHaTrackerStatus::new(mimir)
        .execute(&ctx())
        .await
        .unwrap();
    assert_eq!(result["clusters"][0]["cluster"], "cluster-1");
}

#[tokio::test]
async fn get_distributor_ring_with_http_error() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/distributor/ring"))
        .respond_with(ResponseTemplate::new(503).set_body_string("service unavailable"))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let err = GetDistributorRing::new(mimir)
        .execute(&ctx())
        .await
        .unwrap_err();
    match err {
        ironflow_core::error::OperationError::Http { status, message } => {
            assert_eq!(status, Some(503));
            assert_eq!(message, "service unavailable");
        }
        other => panic!("expected Http error, got: {other}"),
    }
}
