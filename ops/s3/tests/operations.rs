//! Integration tests for S3 operations.
//!
//! These tests use a real S3-compatible server (LocalStack or MinIO).
//! They are marked `#[ignore]` by default and run only when the
//! `S3_TEST_ENDPOINT` environment variable is set.

use std::sync::Arc;

use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_s3::S3Client;
use ironflow_ops_s3::buckets::{CreateBucket, DeleteBucket, HeadBucket, ListBuckets};
use ironflow_ops_s3::objects::{
    CopyObject, DeleteObject, GetObject, HeadObject, ListObjects, PutObject,
};
use ironflow_ops_s3::presigned::PresignGetObject;

fn test_client() -> Option<S3Client> {
    let endpoint = std::env::var("S3_TEST_ENDPOINT").ok()?;
    let access_key = std::env::var("S3_TEST_ACCESS_KEY").unwrap_or_else(|_| "test".to_string());
    let secret_key = std::env::var("S3_TEST_SECRET_KEY").unwrap_or_else(|_| "test".to_string());
    let region = std::env::var("S3_TEST_REGION").unwrap_or_else(|_| "us-east-1".to_string());
    S3Client::from_credentials(&access_key, &secret_key, &region, Some(&endpoint)).ok()
}

fn test_ctx() -> OperationContext {
    OperationContext::new(Arc::new(NoopSecretResolver))
}

#[tokio::test]
#[ignore]
async fn put_object_returns_etag() {
    let s3 = test_client().expect("S3_TEST_ENDPOINT not set");
    let ctx = test_ctx();
    let bucket = "test-ops-put";

    CreateBucket::new(&s3, bucket, None)
        .execute(&ctx)
        .await
        .unwrap();

    let op = PutObject::new(&s3, bucket, "hello.txt", b"hello world".to_vec());
    let result = op.execute(&ctx).await.unwrap();
    assert!(result.get("etag").is_some());

    DeleteObject::new(&s3, bucket, "hello.txt")
        .execute(&ctx)
        .await
        .unwrap();
    DeleteBucket::new(&s3, bucket).execute(&ctx).await.unwrap();
}

#[tokio::test]
#[ignore]
async fn get_object_returns_body_and_metadata() {
    let s3 = test_client().expect("S3_TEST_ENDPOINT not set");
    let bucket = "test-ops-get";

    CreateBucket::new(&s3, bucket, None).run().await.unwrap();

    PutObject::new(&s3, bucket, "data.txt", b"test data".to_vec())
        .run()
        .await
        .unwrap();

    let output = GetObject::new(&s3, bucket, "data.txt").run().await.unwrap();
    assert_eq!(output.body, b"test data");
    assert!(output.content_length.is_some());

    DeleteObject::new(&s3, bucket, "data.txt")
        .run()
        .await
        .unwrap();
    DeleteBucket::new(&s3, bucket).run().await.unwrap();
}

#[tokio::test]
#[ignore]
async fn head_object_returns_metadata_without_body() {
    let s3 = test_client().expect("S3_TEST_ENDPOINT not set");
    let bucket = "test-ops-head";

    CreateBucket::new(&s3, bucket, None).run().await.unwrap();

    PutObject::new(&s3, bucket, "meta.txt", b"some content".to_vec())
        .run()
        .await
        .unwrap();

    let output = HeadObject::new(&s3, bucket, "meta.txt")
        .run()
        .await
        .unwrap();
    assert!(output.content_length.is_some());
    assert!(output.etag.is_some());

    DeleteObject::new(&s3, bucket, "meta.txt")
        .run()
        .await
        .unwrap();
    DeleteBucket::new(&s3, bucket).run().await.unwrap();
}

#[tokio::test]
#[ignore]
async fn delete_object_succeeds() {
    let s3 = test_client().expect("S3_TEST_ENDPOINT not set");
    let ctx = test_ctx();
    let bucket = "test-ops-del";

    CreateBucket::new(&s3, bucket, None)
        .execute(&ctx)
        .await
        .unwrap();

    PutObject::new(&s3, bucket, "del.txt", b"to delete".to_vec())
        .execute(&ctx)
        .await
        .unwrap();

    let result = DeleteObject::new(&s3, bucket, "del.txt")
        .execute(&ctx)
        .await;
    assert!(result.is_ok());

    DeleteBucket::new(&s3, bucket).execute(&ctx).await.unwrap();
}

#[tokio::test]
#[ignore]
async fn list_objects_with_pagination() {
    let s3 = test_client().expect("S3_TEST_ENDPOINT not set");
    let bucket = "test-ops-list";

    CreateBucket::new(&s3, bucket, None).run().await.unwrap();

    for i in 0..3 {
        PutObject::new(
            &s3,
            bucket,
            &format!("item-{i}.txt"),
            format!("data-{i}").into_bytes(),
        )
        .run()
        .await
        .unwrap();
    }

    let output = ListObjects::new(&s3, bucket)
        .with_max_keys(2)
        .run()
        .await
        .unwrap();
    assert_eq!(output.contents.len(), 2);
    assert!(output.is_truncated);
    assert!(output.next_continuation_token.is_some());

    let page2 = ListObjects::new(&s3, bucket)
        .with_continuation_token(output.next_continuation_token.as_ref().unwrap())
        .run()
        .await
        .unwrap();
    assert_eq!(page2.contents.len(), 1);

    for i in 0..3 {
        DeleteObject::new(&s3, bucket, &format!("item-{i}.txt"))
            .run()
            .await
            .unwrap();
    }
    DeleteBucket::new(&s3, bucket).run().await.unwrap();
}

#[tokio::test]
#[ignore]
async fn copy_object_between_keys() {
    let s3 = test_client().expect("S3_TEST_ENDPOINT not set");
    let bucket = "test-ops-copy";

    CreateBucket::new(&s3, bucket, None).run().await.unwrap();

    PutObject::new(&s3, bucket, "src.txt", b"copy me".to_vec())
        .run()
        .await
        .unwrap();

    let output = CopyObject::new(&s3, bucket, "src.txt", bucket, "dst.txt")
        .run()
        .await
        .unwrap();
    assert!(output.etag.is_some());

    let get = GetObject::new(&s3, bucket, "dst.txt").run().await.unwrap();
    assert_eq!(get.body, b"copy me");

    DeleteObject::new(&s3, bucket, "src.txt")
        .run()
        .await
        .unwrap();
    DeleteObject::new(&s3, bucket, "dst.txt")
        .run()
        .await
        .unwrap();
    DeleteBucket::new(&s3, bucket).run().await.unwrap();
}

#[tokio::test]
#[ignore]
async fn get_nonexistent_key_returns_error() {
    let s3 = test_client().expect("S3_TEST_ENDPOINT not set");
    let bucket = "test-ops-err";

    CreateBucket::new(&s3, bucket, None).run().await.unwrap();

    let err = GetObject::new(&s3, bucket, "does-not-exist.txt")
        .run()
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("NoSuchKey") || err.to_string().contains("S3 error"),
        "expected NoSuchKey error, got: {err}"
    );

    DeleteBucket::new(&s3, bucket).run().await.unwrap();
}

#[tokio::test]
#[ignore]
async fn bucket_crud_lifecycle() {
    let s3 = test_client().expect("S3_TEST_ENDPOINT not set");
    let ctx = test_ctx();
    let bucket = "test-ops-bucket-crud";

    let create = CreateBucket::new(&s3, bucket, None).execute(&ctx).await;
    assert!(create.is_ok());

    let head = HeadBucket::new(&s3, bucket).execute(&ctx).await;
    assert!(head.is_ok());

    let list = ListBuckets::new(&s3).run().await.unwrap();
    assert!(
        list.buckets
            .iter()
            .any(|b| b.name.as_deref() == Some(bucket))
    );

    let delete = DeleteBucket::new(&s3, bucket).execute(&ctx).await;
    assert!(delete.is_ok());
}

#[tokio::test]
async fn presign_get_generates_url() {
    let s3 =
        S3Client::from_credentials("AKID", "SECRET", "us-east-1", Some("http://localhost:9999"))
            .unwrap();

    let output = PresignGetObject::new(&s3, "any-bucket", "any-key", 3600)
        .run()
        .await
        .unwrap();
    assert!(output.url.starts_with("http"));
    assert!(output.url.contains("any-bucket"));
    assert!(output.url.contains("any-key"));
    assert_eq!(output.expires_in_secs, 3600);
}
