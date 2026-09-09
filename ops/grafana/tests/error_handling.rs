use std::sync::Arc;

use ironflow_core::error::OperationError;
use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_grafana::GrafanaClient;
use ironflow_ops_grafana::annotations::{AnnotationCreate, AnnotationDelete, AnnotationPatch};
use ironflow_ops_grafana::dashboards::DashboardGet;
use ironflow_ops_grafana::folders::{FolderDelete, FolderUpdate};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn setup() -> (MockServer, GrafanaClient, OperationContext) {
    let server = MockServer::start().await;
    let client = GrafanaClient::new("test-token", &server.uri()).unwrap();
    let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
    (server, client, ctx)
}

#[tokio::test]
async fn http_error_maps_to_operation_error_with_status() {
    let (server, client, ctx) = setup().await;

    Mock::given(method("GET"))
        .and(path("/api/dashboards/uid/bad"))
        .respond_with(ResponseTemplate::new(500).set_body_string("internal server error"))
        .expect(1)
        .mount(&server)
        .await;

    let op = DashboardGet::new(&client, "bad");
    let err = op.execute(&ctx).await.unwrap_err();

    match err {
        OperationError::Http { status, message } => {
            assert_eq!(status, Some(500));
            assert!(message.contains("internal server error"));
        }
        other => panic!("expected Http error, got: {other}"),
    }
}

#[tokio::test]
async fn not_found_maps_to_404() {
    let (server, client, ctx) = setup().await;

    Mock::given(method("GET"))
        .and(path("/api/dashboards/uid/nonexistent"))
        .respond_with(
            ResponseTemplate::new(404).set_body_json(json!({"message": "Dashboard not found"})),
        )
        .expect(1)
        .mount(&server)
        .await;

    let op = DashboardGet::new(&client, "nonexistent");
    let err = op.execute(&ctx).await.unwrap_err();

    match err {
        OperationError::Http { status, message } => {
            assert_eq!(status, Some(404));
            assert!(message.contains("not found"), "message was: {message}");
        }
        other => panic!("expected Http error, got: {other}"),
    }
}

#[tokio::test]
async fn non_json_error_body_still_captured() {
    let (server, client, ctx) = setup().await;

    Mock::given(method("GET"))
        .and(path("/api/dashboards/uid/proxy-err"))
        .respond_with(ResponseTemplate::new(502).set_body_string("<html>Bad Gateway</html>"))
        .expect(1)
        .mount(&server)
        .await;

    let op = DashboardGet::new(&client, "proxy-err");
    let err = op.execute(&ctx).await.unwrap_err();

    match err {
        OperationError::Http { status, message } => {
            assert_eq!(status, Some(502));
            assert!(message.contains("Bad Gateway"));
        }
        other => panic!("expected Http error, got: {other}"),
    }
}

#[tokio::test]
async fn connection_refused_maps_to_http_error() {
    let client = GrafanaClient::new("token", "http://127.0.0.1:1").unwrap();
    let ctx = OperationContext::new(Arc::new(NoopSecretResolver));

    let op = DashboardGet::new(&client, "any");
    let err = op.execute(&ctx).await.unwrap_err();

    match err {
        OperationError::Http { status, message } => {
            assert!(status.is_none());
            assert!(!message.is_empty());
        }
        other => panic!("expected Http error, got: {other}"),
    }
}

// -- Trou 1: 204 No Content handling --

#[tokio::test]
async fn delete_204_no_content_succeeds() {
    let (server, client, ctx) = setup().await;

    Mock::given(method("DELETE"))
        .and(path("/api/folders/no-body"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    let op = FolderDelete::new(&client, "no-body");
    let result = op.execute(&ctx).await.unwrap();
    assert!(result.is_null());
}

#[tokio::test]
async fn delete_204_no_content_for_annotations() {
    let (server, client, ctx) = setup().await;

    Mock::given(method("DELETE"))
        .and(path("/api/annotations/99"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    let op = AnnotationDelete::new(&client, 99);
    let result = op.execute(&ctx).await.unwrap();
    assert!(result.is_null());
}

// -- Trou 2: error paths for non-GET methods --

#[tokio::test]
async fn post_500_maps_to_operation_error() {
    let (server, client, ctx) = setup().await;

    Mock::given(method("POST"))
        .and(path("/api/annotations"))
        .respond_with(ResponseTemplate::new(500).set_body_string("internal server error on create"))
        .expect(1)
        .mount(&server)
        .await;

    let op = AnnotationCreate::new(&client, json!({"text": "test"}));
    let err = op.execute(&ctx).await.unwrap_err();

    match err {
        OperationError::Http { status, message } => {
            assert_eq!(status, Some(500));
            assert!(message.contains("internal server error on create"));
        }
        other => panic!("expected Http error, got: {other}"),
    }
}

#[tokio::test]
async fn put_403_maps_to_forbidden() {
    let (server, client, ctx) = setup().await;

    Mock::given(method("PUT"))
        .and(path("/api/folders/restricted"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({"message": "Access denied"})))
        .expect(1)
        .mount(&server)
        .await;

    let op = FolderUpdate::new(&client, "restricted", "Renamed", 1);
    let err = op.execute(&ctx).await.unwrap_err();

    match err {
        OperationError::Http { status, message } => {
            assert_eq!(status, Some(403));
            assert!(message.contains("Access denied"));
        }
        other => panic!("expected Http error, got: {other}"),
    }
}

#[tokio::test]
async fn delete_404_maps_to_not_found() {
    let (server, client, ctx) = setup().await;

    Mock::given(method("DELETE"))
        .and(path("/api/folders/nonexistent"))
        .respond_with(
            ResponseTemplate::new(404).set_body_json(json!({"message": "Folder not found"})),
        )
        .expect(1)
        .mount(&server)
        .await;

    let op = FolderDelete::new(&client, "nonexistent");
    let err = op.execute(&ctx).await.unwrap_err();

    match err {
        OperationError::Http { status, message } => {
            assert_eq!(status, Some(404));
            assert!(message.contains("not found"), "message was: {message}");
        }
        other => panic!("expected Http error, got: {other}"),
    }
}

#[tokio::test]
async fn patch_422_maps_to_unprocessable() {
    let (server, client, ctx) = setup().await;

    Mock::given(method("PATCH"))
        .and(path("/api/annotations/1"))
        .respond_with(
            ResponseTemplate::new(422).set_body_json(json!({"message": "Invalid annotation data"})),
        )
        .expect(1)
        .mount(&server)
        .await;

    let op = AnnotationPatch::new(&client, 1, json!({"text": "bad"}));
    let err = op.execute(&ctx).await.unwrap_err();

    match err {
        OperationError::Http { status, message } => {
            assert_eq!(status, Some(422));
            assert!(message.contains("Invalid annotation data"));
        }
        other => panic!("expected Http error, got: {other}"),
    }
}
