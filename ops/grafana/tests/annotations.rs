use std::sync::Arc;

use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_grafana::GrafanaClient;
use ironflow_ops_grafana::annotations::{
    AnnotationCreate, AnnotationDelete, AnnotationGetById, AnnotationGetTags, AnnotationList,
    AnnotationPatch, AnnotationUpdate,
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
async fn annotation_create() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("POST"))
        .and(path("/api/annotations"))
        .and(bearer_token("test-token"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"id": 1, "message": "created"})),
        )
        .expect(1)
        .mount(&server)
        .await;

    let body = json!({"text": "deploy v1.0", "dashboardId": 1, "time": 1000});
    let op = AnnotationCreate::new(&client, body);
    assert_eq!(op.kind(), "grafana");
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["id"], 1);
}

#[tokio::test]
async fn annotation_create_run_typed() {
    let (server, client, _ctx) = setup().await;
    Mock::given(method("POST"))
        .and(path("/api/annotations"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": 42, "message": "ok"})))
        .mount(&server)
        .await;

    let op = AnnotationCreate::new(&client, json!({"text": "test"}));
    let output = op.run().await.unwrap();
    assert_eq!(output.id, Some(42));
}

#[tokio::test]
async fn annotation_list_no_params() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/annotations"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id": 1, "text": "a1", "time": 1000}
        ])))
        .expect(1)
        .mount(&server)
        .await;

    let op = AnnotationList::new(&client, None, None);
    let result = op.execute(&ctx).await.unwrap();
    assert!(result.is_array());
    assert_eq!(result[0]["id"], 1);
}

#[tokio::test]
async fn annotation_list_with_from_to() {
    let (server, client, _ctx) = setup().await;
    Mock::given(method("GET"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id": 2, "text": "filtered", "time": 2000}
        ])))
        .mount(&server)
        .await;

    let op = AnnotationList::new(&client, Some(1000), Some(3000));
    let output = op.run().await.unwrap();
    assert_eq!(output.len(), 1);
    assert_eq!(output[0].id, Some(2));
}

#[tokio::test]
async fn annotation_get_by_id() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/annotations/5"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 5, "text": "found", "time": 1000
        })))
        .expect(1)
        .mount(&server)
        .await;

    let op = AnnotationGetById::new(&client, 5);
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["id"], 5);
}

#[tokio::test]
async fn annotation_get_by_id_run_typed() {
    let (server, client, _ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/annotations/7"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 7, "text": "typed", "tags": ["deploy"], "time": 2000, "timeEnd": 3000
        })))
        .mount(&server)
        .await;

    let op = AnnotationGetById::new(&client, 7);
    let output = op.run().await.unwrap();
    assert_eq!(output.id, Some(7));
    assert_eq!(output.text.as_deref(), Some("typed"));
    assert_eq!(output.time_end, Some(3000));
}

#[tokio::test]
async fn annotation_update() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("PUT"))
        .and(path("/api/annotations/3"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"message": "updated"})))
        .expect(1)
        .mount(&server)
        .await;

    let op = AnnotationUpdate::new(&client, 3, json!({"text": "updated text"}));
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["message"], "updated");
}

#[tokio::test]
async fn annotation_patch() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("PATCH"))
        .and(path("/api/annotations/4"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"message": "patched"})))
        .expect(1)
        .mount(&server)
        .await;

    let op = AnnotationPatch::new(&client, 4, json!({"text": "partial update"}));
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["message"], "patched");
}

#[tokio::test]
async fn annotation_delete() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("DELETE"))
        .and(path("/api/annotations/6"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"message": "deleted"})))
        .expect(1)
        .mount(&server)
        .await;

    let op = AnnotationDelete::new(&client, 6);
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["message"], "deleted");
}

#[tokio::test]
async fn annotation_get_tags() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/annotations/tags"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "result": [{"tag": "deploy", "count": 5}]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let op = AnnotationGetTags::new(&client);
    let result = op.execute(&ctx).await.unwrap();
    assert!(result["result"].is_array());
}
