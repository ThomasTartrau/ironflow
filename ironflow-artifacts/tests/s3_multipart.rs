#![cfg(feature = "storage-s3")]

//! `S3BlobStore::put` against a wiremock S3 endpoint: the real store and the
//! real AWS SDK talk HTTP to the double, which only plays the S3 wire protocol.

use std::future::Future;
use std::time::Duration;

use bytes::Bytes;
use futures_util::stream::iter;
use sha2::{Digest, Sha256};
use tokio::time::timeout;
use wiremock::matchers::{method, path, query_param, query_param_exists};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use ironflow_artifacts::blob_store::{BlobDigest, BlobStore, ByteStream};
use ironflow_artifacts::error::ArtifactError;
use ironflow_artifacts::s3::S3BlobStore;

const MIB: usize = 1024 * 1024;
/// Mirrors `PART_SIZE` in `ironflow-artifacts/src/s3.rs`.
const PART_SIZE: usize = 8 * MIB;
const KEY: &str = "artifacts/run/step/id";
const OBJECT_PATH: &str = "/bucket/artifacts/run/step/id";

fn store_for(server: &MockServer) -> S3BlobStore {
    S3BlobStore::from_credentials("bucket", "AKID", "SECRET", "us-east-1", Some(&server.uri()))
        .expect("build store")
}

/// `total` bytes in chunks of 1 MiB, each chunk filled with its own index.
fn chunks_of(total: usize) -> Vec<Bytes> {
    let mut chunks = Vec::new();
    let mut remaining = total;
    let mut index = 0u8;
    while remaining > 0 {
        let len = remaining.min(MIB);
        chunks.push(Bytes::from(vec![index; len]));
        remaining -= len;
        index = index.wrapping_add(1);
    }
    chunks
}

fn stream_of(items: Vec<Result<Bytes, ArtifactError>>) -> ByteStream {
    Box::pin(iter(items))
}

fn ok_stream(chunks: &[Bytes]) -> ByteStream {
    stream_of(chunks.iter().cloned().map(Ok).collect())
}

fn digest_of(chunks: &[Bytes]) -> BlobDigest {
    let mut hasher = Sha256::new();
    let mut size_bytes = 0u64;
    for chunk in chunks {
        hasher.update(chunk);
        size_bytes += chunk.len() as u64;
    }
    BlobDigest {
        size_bytes,
        sha256: hex::encode(hasher.finalize()),
    }
}

async fn mount_create(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path(OBJECT_PATH))
        .and(query_param_exists("uploads"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
             <InitiateMultipartUploadResult>\
             <Bucket>bucket</Bucket><Key>artifacts/run/step/id</Key>\
             <UploadId>UP1</UploadId></InitiateMultipartUploadResult>",
        ))
        .mount(server)
        .await;
}

async fn mount_part(server: &MockServer, number: u32, status: u16) {
    Mock::given(method("PUT"))
        .and(path(OBJECT_PATH))
        .and(query_param("partNumber", number.to_string()))
        .and(query_param("uploadId", "UP1"))
        .respond_with(
            ResponseTemplate::new(status).insert_header("ETag", format!("\"etag-{number}\"")),
        )
        .mount(server)
        .await;
}

async fn mount_complete(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path(OBJECT_PATH))
        .and(query_param("uploadId", "UP1"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
             <CompleteMultipartUploadResult>\
             <Location>http://localhost/bucket/artifacts/run/step/id</Location>\
             <Bucket>bucket</Bucket><Key>artifacts/run/step/id</Key>\
             <ETag>\"etag-final\"</ETag></CompleteMultipartUploadResult>",
        ))
        .mount(server)
        .await;
}

async fn mount_abort(server: &MockServer, status: u16, expected: Option<u64>) {
    let mock = Mock::given(method("DELETE"))
        .and(path(OBJECT_PATH))
        .and(query_param("uploadId", "UP1"))
        .respond_with(ResponseTemplate::new(status));
    match expected {
        Some(times) => mock.expect(times).mount(server).await,
        None => mock.mount(server).await,
    }
}

/// Mounted last: it also matches part requests, which must be mounted before it.
async fn mount_put_object(server: &MockServer) {
    Mock::given(method("PUT"))
        .and(path(OBJECT_PATH))
        .respond_with(ResponseTemplate::new(200))
        .mount(server)
        .await;
}

fn label(request: &Request) -> String {
    let query: Vec<(String, String)> = request
        .url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    let find = |name: &str| {
        query
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
    };

    match request.method.as_str() {
        "POST" if find("uploads").is_some() => "create".to_string(),
        "POST" if find("uploadId").is_some() => "complete".to_string(),
        "PUT" => match find("partNumber") {
            Some(number) => format!("part{number}"),
            None => "put".to_string(),
        },
        "DELETE" => "abort".to_string(),
        other => other.to_string(),
    }
}

async fn received(server: &MockServer) -> Vec<Request> {
    server
        .received_requests()
        .await
        .expect("request recording enabled")
}

async fn labels(server: &MockServer) -> Vec<String> {
    received(server).await.iter().map(label).collect()
}

async fn with_timeout<F: Future<Output = ()>>(body: F) {
    timeout(Duration::from_secs(30), body)
        .await
        .expect("test timed out");
}

#[tokio::test]
async fn s3_put_large_payload_is_uploaded_in_parts_with_unchanged_digest() {
    with_timeout(async {
        let server = MockServer::start().await;
        mount_create(&server).await;
        for number in 1..=3 {
            mount_part(&server, number, 200).await;
        }
        mount_complete(&server).await;
        mount_abort(&server, 204, None).await;

        let chunks = chunks_of(17 * MIB);
        let digest = store_for(&server)
            .put(KEY, ok_stream(&chunks))
            .await
            .expect("put");

        assert_eq!(digest, digest_of(&chunks));
        assert_eq!(digest.size_bytes, (17 * MIB) as u64);
        assert_eq!(
            labels(&server).await,
            ["create", "part1", "part2", "part3", "complete"]
        );

        let requests = received(&server).await;
        for request in &requests[1..3] {
            assert!(request.body.len() >= PART_SIZE, "non-last part too small");
        }
        let body = String::from_utf8(requests[4].body.clone()).expect("complete body is XML");
        let mut cursor = 0;
        for number in 1..=3 {
            let marker = format!("<PartNumber>{number}</PartNumber>");
            let at = body[cursor..]
                .find(&marker)
                .unwrap_or_else(|| panic!("part {number} missing or out of order in {body}"));
            cursor += at;
            let etag = format!("etag-{number}");
            let at = body[cursor..]
                .find(&etag)
                .unwrap_or_else(|| panic!("{etag} missing or out of order in {body}"));
            cursor += at;
        }
    })
    .await;
}

#[tokio::test]
async fn s3_put_exact_part_size_completes_without_empty_trailing_part() {
    with_timeout(async {
        let server = MockServer::start().await;
        mount_create(&server).await;
        mount_part(&server, 1, 200).await;
        mount_part(&server, 2, 200).await;
        mount_complete(&server).await;

        let chunks = chunks_of(PART_SIZE);
        let digest = store_for(&server)
            .put(KEY, ok_stream(&chunks))
            .await
            .expect("put");

        assert_eq!(digest, digest_of(&chunks));
        assert_eq!(labels(&server).await, ["create", "part1", "complete"]);
    })
    .await;
}

#[tokio::test]
async fn s3_put_small_payload_uses_single_put_object() {
    with_timeout(async {
        let server = MockServer::start().await;
        mount_put_object(&server).await;

        let chunks = chunks_of(1024);
        let digest = store_for(&server)
            .put(KEY, ok_stream(&chunks))
            .await
            .expect("put");

        assert_eq!(digest, digest_of(&chunks));
        assert_eq!(labels(&server).await, ["put"]);
    })
    .await;
}

#[tokio::test]
async fn s3_put_empty_payload_uses_single_put_object() {
    with_timeout(async {
        let server = MockServer::start().await;
        mount_put_object(&server).await;

        let digest = store_for(&server)
            .put(KEY, ok_stream(&[]))
            .await
            .expect("put");

        assert_eq!(digest, digest_of(&[]));
        assert_eq!(digest.size_bytes, 0);
        assert_eq!(labels(&server).await, ["put"]);
    })
    .await;
}

#[tokio::test]
async fn s3_put_past_max_bytes_aborts_the_multipart_upload() {
    with_timeout(async {
        let server = MockServer::start().await;
        mount_create(&server).await;
        mount_part(&server, 1, 200).await;
        mount_complete(&server).await;
        mount_abort(&server, 204, Some(1)).await;

        let chunks = chunks_of(12 * MIB);
        let err = store_for(&server)
            .max_bytes((10 * MIB) as u64)
            .put(KEY, ok_stream(&chunks))
            .await
            .expect_err("payload over the limit");

        assert!(matches!(
            err,
            ArtifactError::TooLarge { limit_bytes } if limit_bytes == (10 * MIB) as u64
        ));
        assert_eq!(labels(&server).await, ["create", "part1", "abort"]);
    })
    .await;
}

#[tokio::test]
async fn s3_put_past_max_bytes_before_first_part_starts_no_multipart() {
    with_timeout(async {
        let server = MockServer::start().await;

        let chunks = chunks_of(2048);
        let err = store_for(&server)
            .max_bytes(1024)
            .put(KEY, ok_stream(&chunks))
            .await
            .expect_err("payload over the limit");

        assert!(matches!(err, ArtifactError::TooLarge { limit_bytes: 1024 }));
        assert!(received(&server).await.is_empty());
    })
    .await;
}

#[tokio::test]
async fn s3_put_stream_error_after_first_part_aborts_the_multipart_upload() {
    with_timeout(async {
        let server = MockServer::start().await;
        mount_create(&server).await;
        mount_part(&server, 1, 200).await;
        mount_complete(&server).await;
        mount_abort(&server, 204, Some(1)).await;

        let mut items: Vec<Result<Bytes, ArtifactError>> =
            chunks_of(9 * MIB).into_iter().map(Ok).collect();
        items.push(Err(ArtifactError::Io("source vanished".to_string())));

        let err = store_for(&server)
            .put(KEY, stream_of(items))
            .await
            .expect_err("stream error");

        assert!(matches!(&err, ArtifactError::Io(msg) if msg == "source vanished"));
        assert_eq!(labels(&server).await, ["create", "part1", "abort"]);
    })
    .await;
}

#[tokio::test]
async fn s3_put_upload_part_failure_aborts_and_returns_io_error() {
    with_timeout(async {
        let server = MockServer::start().await;
        mount_create(&server).await;
        // 400 is not retried by the SDK, so the test does not wait on backoff.
        mount_part(&server, 1, 400).await;
        mount_abort(&server, 204, Some(1)).await;

        let chunks = chunks_of(9 * MIB);
        let err = store_for(&server)
            .put(KEY, ok_stream(&chunks))
            .await
            .expect_err("part upload fails");

        assert!(matches!(&err, ArtifactError::Io(msg) if msg.contains("UploadPart")));
        assert_eq!(labels(&server).await, ["create", "part1", "abort"]);
    })
    .await;
}

#[tokio::test]
async fn s3_put_abort_failure_does_not_mask_too_large() {
    with_timeout(async {
        let server = MockServer::start().await;
        mount_create(&server).await;
        mount_part(&server, 1, 200).await;
        mount_abort(&server, 400, Some(1)).await;

        let chunks = chunks_of(12 * MIB);
        let err = store_for(&server)
            .max_bytes((10 * MIB) as u64)
            .put(KEY, ok_stream(&chunks))
            .await
            .expect_err("payload over the limit");

        assert!(matches!(err, ArtifactError::TooLarge { .. }));
    })
    .await;
}

#[tokio::test]
async fn s3_put_complete_failure_aborts_and_returns_io_error() {
    with_timeout(async {
        let server = MockServer::start().await;
        mount_create(&server).await;
        mount_part(&server, 1, 200).await;
        mount_part(&server, 2, 200).await;
        Mock::given(method("POST"))
            .and(path(OBJECT_PATH))
            .and(query_param("uploadId", "UP1"))
            .respond_with(ResponseTemplate::new(400))
            .mount(&server)
            .await;
        mount_abort(&server, 204, Some(1)).await;

        let chunks = chunks_of(9 * MIB);
        let err = store_for(&server)
            .put(KEY, ok_stream(&chunks))
            .await
            .expect_err("complete fails");

        assert!(matches!(&err, ArtifactError::Io(msg) if msg.contains("CompleteMultipartUpload")));
        assert_eq!(
            labels(&server).await,
            ["create", "part1", "part2", "complete", "abort"]
        );
    })
    .await;
}

#[tokio::test]
async fn s3_put_create_response_without_upload_id_returns_io_error() {
    with_timeout(async {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(OBJECT_PATH))
            .and(query_param_exists("uploads"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
                 <InitiateMultipartUploadResult>\
                 <Bucket>bucket</Bucket><Key>artifacts/run/step/id</Key>\
                 </InitiateMultipartUploadResult>",
            ))
            .mount(&server)
            .await;

        let chunks = chunks_of(9 * MIB);
        let err = store_for(&server)
            .put(KEY, ok_stream(&chunks))
            .await
            .expect_err("no upload id");

        assert!(matches!(err, ArtifactError::Io(_)));
        assert_eq!(labels(&server).await, ["create"]);
    })
    .await;
}

#[tokio::test]
async fn s3_put_part_response_without_etag_aborts_and_returns_io_error() {
    with_timeout(async {
        let server = MockServer::start().await;
        mount_create(&server).await;
        Mock::given(method("PUT"))
            .and(path(OBJECT_PATH))
            .and(query_param("partNumber", "1"))
            .and(query_param("uploadId", "UP1"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        mount_abort(&server, 204, Some(1)).await;

        let chunks = chunks_of(9 * MIB);
        let err = store_for(&server)
            .put(KEY, ok_stream(&chunks))
            .await
            .expect_err("no etag");

        assert!(matches!(err, ArtifactError::Io(_)));
        assert_eq!(labels(&server).await, ["create", "part1", "abort"]);
    })
    .await;
}
