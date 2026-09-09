use std::sync::Arc;

use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_loki::LokiClient;
use ironflow_ops_loki::labels::GetLabels;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn ctx() -> OperationContext {
    OperationContext::new(Arc::new(NoopSecretResolver))
}

#[tokio::test]
async fn bearer_token_is_sent_in_authorization_header() {
    let server = MockServer::start().await;
    let body = serde_json::json!({"status": "success", "data": ["job"]});

    Mock::given(method("GET"))
        .and(path("/loki/api/v1/labels"))
        .and(header("Authorization", "Bearer my-secret-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let loki =
        LokiClient::new(&server.uri(), reqwest::Client::new()).with_bearer_token("my-secret-token");
    let op = GetLabels::new(loki);

    let result = op.execute(&ctx()).await.unwrap();
    assert_eq!(result["status"], "success");
}

#[tokio::test]
async fn basic_auth_is_sent_in_authorization_header() {
    let server = MockServer::start().await;
    let body = serde_json::json!({"status": "success", "data": ["job"]});

    // reqwest encodes basic auth as base64("admin:secret") = "YWRtaW46c2VjcmV0"
    Mock::given(method("GET"))
        .and(path("/loki/api/v1/labels"))
        .and(header("Authorization", "Basic YWRtaW46c2VjcmV0"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let loki =
        LokiClient::new(&server.uri(), reqwest::Client::new()).with_basic_auth("admin", "secret");
    let op = GetLabels::new(loki);

    let result = op.execute(&ctx()).await.unwrap();
    assert_eq!(result["status"], "success");
}

#[tokio::test]
async fn no_auth_sends_no_authorization_header() {
    let server = MockServer::start().await;
    let body = serde_json::json!({"status": "success", "data": ["job"]});

    Mock::given(method("GET"))
        .and(path("/loki/api/v1/labels"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .expect(1)
        .mount(&server)
        .await;

    let loki = LokiClient::new(&server.uri(), reqwest::Client::new());
    let op = GetLabels::new(loki);

    let result = op.execute(&ctx()).await.unwrap();
    assert_eq!(result["status"], "success");
}
