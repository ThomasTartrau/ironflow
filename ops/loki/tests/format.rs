use std::sync::Arc;

use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_loki::LokiClient;
use ironflow_ops_loki::format::FormatQuery;
use serde_json::json;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn ctx() -> OperationContext {
    OperationContext::new(Arc::new(NoopSecretResolver))
}

#[tokio::test]
async fn format_query_returns_formatted_expression() {
    let server = MockServer::start().await;
    let body = json!({"status": "success", "data": r#"{job="varlogs"} |= "error""#});

    Mock::given(method("GET"))
        .and(path("/loki/api/v1/format_query"))
        .and(query_param("query", r#"{job="varlogs"} |= "error""#))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let loki = LokiClient::new(&server.uri(), reqwest::Client::new());
    let op = FormatQuery::new(loki, r#"{job="varlogs"} |= "error""#);

    let result = op.execute(&ctx()).await.unwrap();
    assert_eq!(result["status"], "success");
}
