use std::sync::Arc;

use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_mimir::MimirClient;
use ironflow_ops_mimir::compactor::{
    FinishBlockUpload, GetRing, GetTenants, StartBlockUpload, UploadBlockFile,
};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn ctx() -> OperationContext {
    OperationContext::new(Arc::new(NoopSecretResolver))
}

#[tokio::test]
async fn get_ring_returns_text_wrapped_in_json() {
    let server = MockServer::start().await;
    let ring_text = "Compactor ring: ACTIVE";

    Mock::given(method("GET"))
        .and(path("/compactor/ring"))
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
    let body = serde_json::json!(["tenant-1", "tenant-2"]);

    Mock::given(method("GET"))
        .and(path("/compactor/tenants"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = GetTenants::new(mimir).execute(&ctx()).await.unwrap();
    assert_eq!(result[0], "tenant-1");
}

#[tokio::test]
async fn start_block_upload_sends_post() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/v1/upload/block/01ABCDEF/start"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = StartBlockUpload::new(mimir, "01ABCDEF")
        .execute(&ctx())
        .await
        .unwrap();
    assert_eq!(result["status"], "success");
}

#[tokio::test]
async fn upload_block_file_sends_post_with_path_query() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/v1/upload/block/01ABCDEF/files"))
        .and(query_param("path", "index"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = UploadBlockFile::new(mimir, "01ABCDEF", "index", vec![1, 2, 3])
        .execute(&ctx())
        .await
        .unwrap();
    assert_eq!(result["status"], "success");
}

#[tokio::test]
async fn finish_block_upload_sends_post() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/v1/upload/block/01ABCDEF/finish"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let result = FinishBlockUpload::new(mimir, "01ABCDEF")
        .execute(&ctx())
        .await
        .unwrap();
    assert_eq!(result["status"], "success");
}

#[tokio::test]
async fn start_block_upload_with_http_error() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/v1/upload/block/01ABCDEF/start"))
        .respond_with(ResponseTemplate::new(409).set_body_string("upload already started"))
        .mount(&server)
        .await;

    let mimir = MimirClient::new(&server.uri(), reqwest::Client::new());
    let err = StartBlockUpload::new(mimir, "01ABCDEF")
        .execute(&ctx())
        .await
        .unwrap_err();
    match err {
        ironflow_core::error::OperationError::Http { status, message } => {
            assert_eq!(status, Some(409));
            assert_eq!(message, "upload already started");
        }
        other => panic!("expected Http error, got: {other}"),
    }
}
