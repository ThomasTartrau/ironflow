//! Upload endpoint: sanitized multipart body, refusal of a content breaking the encoding.

use std::sync::Arc;

use gitlab::api::AsyncQuery;
use gitlab::api::common::NameOrId;
use gitlab::{AsyncGitlab, GitlabBuilder};
use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_gitlab::GitLab;
use ironflow_ops_gitlab::endpoints::uploads::UploadFile;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const MARKER: &str = "--ironflow-ops-gitlab-upload-boundary-7f3a9c1e";

async fn insecure_client(server: &MockServer) -> AsyncGitlab {
    Mock::given(method("GET"))
        .and(path("/api/v4/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": 1})))
        .mount(server)
        .await;

    GitlabBuilder::new(server.address().to_string(), "token")
        .insecure()
        .build_async()
        .await
        .unwrap()
}

async fn mount_upload(server: &MockServer, route: &str) {
    Mock::given(method("POST"))
        .and(path(route))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "alt": "shot",
            "url": "/uploads/abc/shot.png",
            "full_path": "/group/project/uploads/abc/shot.png",
            "markdown": "![shot](/uploads/abc/shot.png)"
        })))
        .mount(server)
        .await;
}

async fn last_post_body(server: &MockServer) -> (String, Vec<u8>) {
    let requests = server.received_requests().await.unwrap();
    let request = requests
        .iter()
        .rev()
        .find(|r| r.method.as_str() == "POST")
        .unwrap();
    let content_type = request
        .headers
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    (content_type, request.body.clone())
}

#[tokio::test]
async fn upload_sends_multipart_file_and_returns_markdown() {
    let server = MockServer::start().await;
    mount_upload(&server, "/api/v4/projects/42/uploads").await;
    let client = insecure_client(&server).await;

    let upload = UploadFile::new(NameOrId::from(42), "shot.png", b"PNGDATA".to_vec()).unwrap();
    let result: Value = upload.query_async(&client).await.unwrap();
    assert_eq!(result["markdown"], "![shot](/uploads/abc/shot.png)");

    let (content_type, body) = last_post_body(&server).await;
    assert!(content_type.starts_with("multipart/form-data; boundary="));
    let expected = format!(
        "{MARKER}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"shot.png\"\r\nContent-Type: application/octet-stream\r\n\r\nPNGDATA\r\n{MARKER}--\r\n"
    );
    assert_eq!(String::from_utf8(body).unwrap(), expected);
}

#[tokio::test]
async fn upload_to_group_project_path_is_percent_encoded() {
    let server = MockServer::start().await;
    mount_upload(&server, "/api/v4/projects/group%2Fproject/uploads").await;
    let client = insecure_client(&server).await;

    let upload =
        UploadFile::new(NameOrId::from("group/project"), "shot.png", b"x".to_vec()).unwrap();
    let result: Value = upload.query_async(&client).await.unwrap();
    assert_eq!(result["alt"], "shot");
}

#[tokio::test]
async fn upload_sanitizes_hostile_filename() {
    let server = MockServer::start().await;
    mount_upload(&server, "/api/v4/projects/42/uploads").await;
    let client = insecure_client(&server).await;

    let upload = UploadFile::new(
        NameOrId::from(42),
        "../../x\"\r\nInjected: 1.png",
        b"x".to_vec(),
    )
    .unwrap();
    let _: Value = upload.query_async(&client).await.unwrap();

    let (_, body) = last_post_body(&server).await;
    let text = String::from_utf8(body).unwrap();
    assert!(text.contains("filename=\"x___Injected__1.png\"\r\n"));
    assert!(!text.contains("Injected: 1"));
}

#[tokio::test]
async fn upload_with_only_dots_filename_falls_back() {
    let server = MockServer::start().await;
    mount_upload(&server, "/api/v4/projects/42/uploads").await;
    let client = insecure_client(&server).await;

    let upload = UploadFile::new(NameOrId::from(42), "...", b"x".to_vec()).unwrap();
    let _: Value = upload.query_async(&client).await.unwrap();

    let (_, body) = last_post_body(&server).await;
    assert!(
        String::from_utf8(body)
            .unwrap()
            .contains("filename=\"upload\"")
    );
}

#[tokio::test]
async fn upload_accepts_empty_content() {
    let server = MockServer::start().await;
    mount_upload(&server, "/api/v4/projects/42/uploads").await;
    let client = insecure_client(&server).await;

    let upload = UploadFile::new(NameOrId::from(42), "empty.txt", Vec::new()).unwrap();
    let result: Value = upload.query_async(&client).await.unwrap();
    assert_eq!(result["alt"], "shot");
}

#[test]
fn upload_refuses_content_containing_the_boundary() {
    let middle = format!("abc{MARKER}def").into_bytes();
    let start = format!("{MARKER}tail").into_bytes();
    let end = format!("head{MARKER}").into_bytes();
    let only = MARKER.as_bytes().to_vec();

    for content in [middle, start, end, only] {
        let err = UploadFile::new(NameOrId::from(42), "a.bin", content)
            .err()
            .unwrap();
        assert!(err.to_string().contains("boundary"));
    }
}

#[tokio::test]
async fn upload_to_missing_project_surfaces_an_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v4/projects/404/uploads"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({"message": "404 Not found"})))
        .mount(&server)
        .await;
    let client = insecure_client(&server).await;

    let upload = UploadFile::new(NameOrId::from(404), "a.png", b"x".to_vec()).unwrap();
    let result: Result<Value, _> = upload.query_async(&client).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn upload_through_an_operation_returns_markdown() {
    let server = MockServer::start().await;
    mount_upload(&server, "/api/v4/projects/42/uploads").await;

    let gitlab: GitLab = insecure_client(&server).await.into();
    let upload = UploadFile::new(NameOrId::from(42), "shot.png", b"PNGDATA".to_vec()).unwrap();
    let ctx = OperationContext::new(Arc::new(NoopSecretResolver));

    let result = gitlab.op(upload).execute(&ctx).await.unwrap();
    assert_eq!(result["markdown"], "![shot](/uploads/abc/shot.png)");

    let (content_type, body) = last_post_body(&server).await;
    assert!(content_type.starts_with("multipart/form-data; boundary="));
    assert!(String::from_utf8(body).unwrap().contains("PNGDATA"));
}
