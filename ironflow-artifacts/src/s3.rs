//! [`S3BlobStore`] -- artifact blobs on an S3-compatible object store.
//!
//! Suitable for multi-worker deployments, container environments, and any
//! topology where the API replicas do not share a filesystem. Compatible with
//! AWS S3, MinIO, Cloudflare R2, and GCS in S3-compatibility mode.
//!
//! Enabled by the `storage-s3` feature flag.

use aws_config::Region;
use aws_credential_types::Credentials;
use aws_sdk_s3::Client;
use aws_sdk_s3::config::Builder;
use aws_sdk_s3::types::{CompletedMultipartUpload, CompletedPart};
use aws_smithy_types::byte_stream::ByteStream as SdkByteStream;
use bytes::{Bytes, BytesMut};
use futures_util::StreamExt;
use futures_util::stream::unfold;
use sha2::{Digest, Sha256};
use tracing::warn;

use crate::blob_store::{BlobDigest, BlobFuture, BlobStore, ByteStream};
use crate::error::ArtifactError;
use crate::local::DEFAULT_MAX_ARTIFACT_BYTES;

/// Size of one multipart part. S3 requires at least 5 MiB for every part but the last.
const PART_SIZE: usize = 8 * 1024 * 1024;

/// S3 hard limit on the number of parts of one multipart upload.
const MAX_PARTS: usize = 10_000;

/// S3-compatible [`BlobStore`].
///
/// Blobs are stored as objects under `{prefix}/{key}` in the configured bucket.
/// SHA-256 is computed chunk by chunk while reading the upload. An upload
/// smaller than one part (8 MiB) is sent with a single `PutObject`; a larger one
/// uses a multipart upload with 8 MiB parts, sent sequentially. Memory per
/// upload is therefore bounded by one part, not by the size limit. The
/// multipart upload is aborted on error or when the size limit is exceeded.
/// Downloads are streamed chunk by chunk from the S3 response body, never
/// buffered whole in memory.
///
/// An abort cannot run if the upload future is dropped mid-way (for example on
/// a process crash). Configure a bucket lifecycle rule
/// `AbortIncompleteMultipartUpload` so orphaned parts do not accumulate.
///
/// # Credentials
///
/// Use [`S3BlobStore::from_env`] to load credentials from the standard AWS SDK
/// chain (environment variables, IAM role, `~/.aws/credentials`), or
/// [`S3BlobStore::from_credentials`] for explicit credentials (useful for
/// MinIO, R2, and GCS S3-compat).
///
/// # Examples
///
/// ```no_run
/// use ironflow_artifacts::s3::S3BlobStore;
///
/// # async fn example() -> Result<(), ironflow_artifacts::error::ArtifactError> {
/// let store = S3BlobStore::from_credentials(
///     "my-bucket",
///     "AKID",
///     "SECRET",
///     "eu-west-1",
///     None,
/// )?;
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct S3BlobStore {
    client: Client,
    bucket: String,
    prefix: String,
    max_bytes: u64,
}

impl std::fmt::Debug for S3BlobStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("S3BlobStore")
            .field("bucket", &self.bucket)
            .field("prefix", &self.prefix)
            .field("max_bytes", &self.max_bytes)
            .field("client", &"[aws_sdk_s3::Client]")
            .finish()
    }
}

impl S3BlobStore {
    /// Build a store using the standard AWS SDK credential chain.
    ///
    /// Reads `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_REGION`, and
    /// optionally `AWS_ENDPOINT_URL` from the environment (or from an IAM role,
    /// `~/.aws/credentials`, etc.).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_artifacts::s3::S3BlobStore;
    ///
    /// # async fn example() {
    /// let store = S3BlobStore::from_env("my-bucket").await;
    /// # }
    /// ```
    pub async fn from_env(bucket: &str) -> Self {
        let config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
        Self {
            client: Client::new(&config),
            bucket: bucket.to_string(),
            prefix: String::new(),
            max_bytes: DEFAULT_MAX_ARTIFACT_BYTES,
        }
    }

    /// Build a store with explicit credentials.
    ///
    /// Pass `endpoint_url` for MinIO, R2, LocalStack, or GCS S3-compat.
    ///
    /// # Errors
    ///
    /// Returns [`ArtifactError::Io`] if the access key or secret is empty.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_artifacts::s3::S3BlobStore;
    ///
    /// # fn example() -> Result<(), ironflow_artifacts::error::ArtifactError> {
    /// let store = S3BlobStore::from_credentials(
    ///     "my-bucket", "AKID", "SECRET", "eu-west-1", None,
    /// )?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn from_credentials(
        bucket: &str,
        access_key_id: &str,
        secret_access_key: &str,
        region: &str,
        endpoint_url: Option<&str>,
    ) -> Result<Self, ArtifactError> {
        if access_key_id.trim().is_empty() {
            return Err(ArtifactError::Io(
                "access_key_id must not be empty".to_string(),
            ));
        }
        if secret_access_key.trim().is_empty() {
            return Err(ArtifactError::Io(
                "secret_access_key must not be empty".to_string(),
            ));
        }

        let creds = Credentials::new(
            access_key_id,
            secret_access_key,
            None,
            None,
            "ironflow-artifacts",
        );

        let mut config = Builder::new()
            .region(Region::new(region.to_string()))
            .credentials_provider(creds)
            .force_path_style(true);

        if let Some(url) = endpoint_url {
            config = config.endpoint_url(url);
        }

        Ok(Self {
            client: Client::from_conf(config.build()),
            bucket: bucket.to_string(),
            prefix: String::new(),
            max_bytes: DEFAULT_MAX_ARTIFACT_BYTES,
        })
    }

    /// Set a key prefix prepended to every storage key.
    ///
    /// Useful for sharing a bucket across environments (e.g. `staging/`).
    pub fn with_prefix(mut self, prefix: &str) -> Self {
        self.prefix = prefix.to_string();
        self
    }

    /// Override the per-artifact size limit.
    pub fn max_bytes(mut self, limit: u64) -> Self {
        self.max_bytes = limit;
        self
    }

    /// The configured per-artifact size limit, in bytes.
    pub fn max_bytes_limit(&self) -> u64 {
        self.max_bytes
    }

    /// The S3 bucket name.
    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    /// The key prefix.
    pub fn key_prefix(&self) -> &str {
        &self.prefix
    }

    /// The underlying S3 client, for operations not covered by [`BlobStore`].
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// Build the full S3 object key from a storage key.
    fn full_key(&self, key: &str) -> String {
        if self.prefix.is_empty() {
            key.to_string()
        } else {
            format!("{}{}", self.prefix, key)
        }
    }
}

impl S3BlobStore {
    /// Upload `content`, aborting the multipart upload if one was started and
    /// the upload failed. The original error is always the one returned.
    async fn put_stream(
        &self,
        key: &str,
        content: ByteStream,
    ) -> Result<BlobDigest, ArtifactError> {
        let full_key = self.full_key(key);
        let mut upload = None;
        let result = self.upload_parts(&full_key, &mut upload, content).await;
        // No abort if the future is dropped: the bucket lifecycle rule is the safety net.
        if let (Err(_), Some(upload_id)) = (&result, &upload) {
            self.abort_multipart(&full_key, upload_id).await;
        }
        result
    }

    /// Read `content` chunk by chunk, sending one part at a time.
    ///
    /// `upload` receives the multipart upload id as soon as it is created, so
    /// the caller can abort it on error.
    async fn upload_parts(
        &self,
        full_key: &str,
        upload: &mut Option<String>,
        mut content: ByteStream,
    ) -> Result<BlobDigest, ArtifactError> {
        let mut hasher = Sha256::new();
        let mut size_bytes: u64 = 0;
        let mut buf = PartBuffer::new(PART_SIZE);
        let mut parts: Vec<CompletedPart> = Vec::new();

        while let Some(chunk) = content.next().await {
            let bytes = chunk?;
            size_bytes += bytes.len() as u64;
            // Checked before buffering so an oversized chunk is never held.
            if size_bytes > self.max_bytes {
                return Err(ArtifactError::TooLarge {
                    limit_bytes: self.max_bytes,
                });
            }
            hasher.update(&bytes);
            buf.push(&bytes);

            while let Some(part) = buf.next_part() {
                let upload_id = match upload.clone() {
                    Some(id) => id,
                    None => {
                        let id = self.create_multipart(full_key).await?;
                        *upload = Some(id.clone());
                        id
                    }
                };
                self.upload_part(full_key, &upload_id, &mut parts, part)
                    .await?;
            }
        }

        let rest = buf.into_rest();
        match upload.clone() {
            None => {
                // Also covers empty content: CompleteMultipartUpload needs at least one part.
                self.client
                    .put_object()
                    .bucket(&self.bucket)
                    .key(full_key)
                    .body(SdkByteStream::from(rest))
                    .send()
                    .await
                    .map_err(|err| ArtifactError::Io(format!("S3 PutObject failed: {err}")))?;
            }
            Some(upload_id) => {
                if !rest.is_empty() {
                    self.upload_part(full_key, &upload_id, &mut parts, rest)
                        .await?;
                }
                self.client
                    .complete_multipart_upload()
                    .bucket(&self.bucket)
                    .key(full_key)
                    .upload_id(&upload_id)
                    .multipart_upload(
                        CompletedMultipartUpload::builder()
                            .set_parts(Some(parts))
                            .build(),
                    )
                    .send()
                    .await
                    .map_err(|err| {
                        ArtifactError::Io(format!("S3 CompleteMultipartUpload failed: {err}"))
                    })?;
            }
        }

        Ok(BlobDigest {
            size_bytes,
            sha256: hex::encode(hasher.finalize()),
        })
    }

    /// Start a multipart upload and return its id.
    async fn create_multipart(&self, full_key: &str) -> Result<String, ArtifactError> {
        let resp = self
            .client
            .create_multipart_upload()
            .bucket(&self.bucket)
            .key(full_key)
            .send()
            .await
            .map_err(|err| ArtifactError::Io(format!("S3 CreateMultipartUpload failed: {err}")))?;
        resp.upload_id().map(String::from).ok_or_else(|| {
            ArtifactError::Io("S3 CreateMultipartUpload returned no upload id".to_string())
        })
    }

    /// Send one part and record its ETag in `parts`.
    async fn upload_part(
        &self,
        full_key: &str,
        upload_id: &str,
        parts: &mut Vec<CompletedPart>,
        part: Bytes,
    ) -> Result<(), ArtifactError> {
        if parts.len() >= MAX_PARTS {
            return Err(ArtifactError::Io(format!(
                "S3 multipart uploads are limited to {MAX_PARTS} parts: the artifact is too large"
            )));
        }
        let part_number = i32::try_from(parts.len() + 1)
            .map_err(|err| ArtifactError::Io(format!("S3 part number out of range: {err}")))?;

        let resp = self
            .client
            .upload_part()
            .bucket(&self.bucket)
            .key(full_key)
            .upload_id(upload_id)
            .part_number(part_number)
            .body(SdkByteStream::from(part))
            .send()
            .await
            .map_err(|err| ArtifactError::Io(format!("S3 UploadPart failed: {err}")))?;
        let e_tag = resp.e_tag().ok_or_else(|| {
            ArtifactError::Io(format!("S3 UploadPart {part_number} returned no ETag"))
        })?;

        parts.push(
            CompletedPart::builder()
                .part_number(part_number)
                .e_tag(e_tag)
                .build(),
        );
        Ok(())
    }

    /// Abort a multipart upload, logging (never returning) a failure so the
    /// error that caused the abort is not masked.
    async fn abort_multipart(&self, full_key: &str, upload_id: &str) {
        let result = self
            .client
            .abort_multipart_upload()
            .bucket(&self.bucket)
            .key(full_key)
            .upload_id(upload_id)
            .send()
            .await;
        if let Err(err) = result {
            warn!(key = full_key, upload_id, error = %err, "S3 AbortMultipartUpload failed");
        }
    }
}

/// Accumulates upload chunks and hands them out as fixed-size parts, so at most
/// one part (plus one incoming chunk) is held in memory.
struct PartBuffer {
    buf: BytesMut,
    part_size: usize,
}

impl PartBuffer {
    fn new(part_size: usize) -> Self {
        Self {
            buf: BytesMut::new(),
            part_size,
        }
    }

    fn push(&mut self, chunk: &[u8]) {
        self.buf.extend_from_slice(chunk);
    }

    /// Take one full part off the front, without copying, if enough is buffered.
    fn next_part(&mut self) -> Option<Bytes> {
        if self.buf.len() >= self.part_size {
            Some(self.buf.split_to(self.part_size).freeze())
        } else {
            None
        }
    }

    /// What remains, shorter than one part once drained.
    fn into_rest(self) -> Bytes {
        self.buf.freeze()
    }
}

impl BlobStore for S3BlobStore {
    fn put<'a>(&'a self, key: &'a str, content: ByteStream) -> BlobFuture<'a, BlobDigest> {
        Box::pin(self.put_stream(key, content))
    }

    fn get<'a>(&'a self, key: &'a str) -> BlobFuture<'a, ByteStream> {
        Box::pin(async move {
            let full_key = self.full_key(key);
            let resp = self
                .client
                .get_object()
                .bucket(&self.bucket)
                .key(&full_key)
                .send()
                .await
                .map_err(|err| {
                    let msg = err.to_string();
                    if msg.contains("NoSuchKey") || msg.contains("not found") {
                        ArtifactError::NotFound(key.to_string())
                    } else {
                        ArtifactError::Io(format!("S3 GetObject failed: {err}"))
                    }
                })?;

            Ok(stream_from_sdk_body(resp.body))
        })
    }

    fn delete<'a>(&'a self, key: &'a str) -> BlobFuture<'a, bool> {
        Box::pin(async move {
            let full_key = self.full_key(key);
            let exists = self
                .client
                .head_object()
                .bucket(&self.bucket)
                .key(&full_key)
                .send()
                .await
                .is_ok();

            if exists {
                self.client
                    .delete_object()
                    .bucket(&self.bucket)
                    .key(&full_key)
                    .send()
                    .await
                    .map_err(|err| ArtifactError::Io(format!("S3 DeleteObject: {err}")))?;
            }
            Ok(exists)
        })
    }

    fn list_keys<'a>(&'a self, prefix: &'a str) -> BlobFuture<'a, Vec<String>> {
        Box::pin(async move {
            let full_prefix = self.full_key(prefix);
            let mut keys = Vec::new();
            let mut continuation_token: Option<String> = None;

            loop {
                let mut req = self
                    .client
                    .list_objects_v2()
                    .bucket(&self.bucket)
                    .prefix(&full_prefix);

                if let Some(token) = &continuation_token {
                    req = req.continuation_token(token);
                }

                let resp = req
                    .send()
                    .await
                    .map_err(|err| ArtifactError::Io(format!("S3 ListObjectsV2: {err}")))?;

                for obj in resp.contents() {
                    if let Some(obj_key) = obj.key() {
                        let stripped = if self.prefix.is_empty() {
                            obj_key.to_string()
                        } else {
                            obj_key
                                .strip_prefix(&self.prefix)
                                .unwrap_or(obj_key)
                                .to_string()
                        };
                        keys.push(stripped);
                    }
                }

                if resp.is_truncated() == Some(true) {
                    continuation_token = resp.next_continuation_token().map(String::from);
                } else {
                    break;
                }
            }
            Ok(keys)
        })
    }
}

/// Adapt an S3 response body into a [`ByteStream`] that yields its chunks as
/// they arrive, so a large object never sits whole in memory.
///
/// A read error is yielded once as [`ArtifactError::Io`], then the stream ends.
fn stream_from_sdk_body(body: SdkByteStream) -> ByteStream {
    Box::pin(unfold(Some(body), |state| async move {
        let mut body = state?;
        match body.try_next().await {
            Ok(Some(chunk)) => Some((Ok(chunk), Some(body))),
            Ok(None) => None,
            Err(err) => {
                let err = ArtifactError::Io(format!("S3 body read: {err}"));
                Some((Err(err), None))
            }
        }
    }))
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::NamedTempFile;

    use super::*;

    #[tokio::test]
    async fn s3_body_stream_yields_chunks_in_order() {
        let payload: Vec<u8> = (0..10_000u32).map(|i| (i % 251) as u8).collect();
        let mut file = NamedTempFile::new().expect("temp file");
        file.write_all(&payload).expect("write payload");
        file.flush().expect("flush payload");

        let body = SdkByteStream::read_from()
            .path(file.path())
            .buffer_size(1024)
            .build()
            .await
            .expect("sdk body from file");

        let mut stream = stream_from_sdk_body(body);
        let mut chunks = 0;
        let mut collected = Vec::new();
        while let Some(chunk) = stream.next().await {
            collected.extend_from_slice(&chunk.expect("chunk"));
            chunks += 1;
        }

        assert!(chunks > 1, "body must be streamed in several chunks");
        assert_eq!(collected, payload);
    }

    #[tokio::test]
    async fn s3_body_stream_empty_body_yields_nothing() {
        let mut stream = stream_from_sdk_body(SdkByteStream::from(Vec::new()));
        while let Some(chunk) = stream.next().await {
            assert!(chunk.expect("chunk").is_empty(), "no data expected");
        }
    }

    fn drain(buf: &mut PartBuffer) -> Vec<Bytes> {
        let mut parts = Vec::new();
        while let Some(part) = buf.next_part() {
            parts.push(part);
        }
        parts
    }

    #[test]
    fn part_buffer_splits_oversized_chunk_into_full_parts() {
        let input: Vec<u8> = (0..10u8).collect();
        let mut buf = PartBuffer::new(4);
        buf.push(&input);

        let parts = drain(&mut buf);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].len(), 4);
        assert_eq!(parts[1].len(), 4);

        let rest = buf.into_rest();
        assert_eq!(rest.len(), 2);

        let mut joined = Vec::new();
        for part in &parts {
            joined.extend_from_slice(part);
        }
        joined.extend_from_slice(&rest);
        assert_eq!(joined, input);
    }

    #[test]
    fn part_buffer_holds_less_than_one_part_after_draining() {
        let mut buf = PartBuffer::new(4);
        let mut held = 0;
        for i in 0..50usize {
            let chunk = vec![0u8; (i % 3) + 1];
            held += chunk.len();
            buf.push(&chunk);
            held -= drain(&mut buf).iter().map(Bytes::len).sum::<usize>();
            assert!(buf.buf.len() < 4);
            assert_eq!(buf.buf.len(), held);
        }
    }

    #[test]
    fn part_buffer_exact_multiple_leaves_empty_rest() {
        let mut buf = PartBuffer::new(4);
        buf.push(&[1u8; 8]);
        assert_eq!(drain(&mut buf).len(), 2);
        assert!(buf.into_rest().is_empty());
    }

    #[test]
    fn part_buffer_empty_input_yields_nothing() {
        let mut buf = PartBuffer::new(4);
        buf.push(&[]);
        assert!(buf.next_part().is_none());
        assert!(buf.into_rest().is_empty());
    }

    #[test]
    fn part_buffer_below_part_size_yields_no_part() {
        let mut buf = PartBuffer::new(4);
        buf.push(&[1, 2, 3]);
        assert!(buf.next_part().is_none());
        assert_eq!(buf.into_rest().as_ref(), &[1, 2, 3]);
    }

    #[test]
    fn s3_blob_store_construction_with_valid_config() {
        let store = S3BlobStore::from_credentials("my-bucket", "AKID", "SECRET", "eu-west-1", None)
            .expect("should build");
        assert_eq!(store.bucket(), "my-bucket");
        assert_eq!(store.max_bytes_limit(), DEFAULT_MAX_ARTIFACT_BYTES);
        assert!(store.key_prefix().is_empty());
    }

    #[test]
    fn s3_blob_store_with_custom_endpoint_for_minio() {
        let store = S3BlobStore::from_credentials(
            "test-bucket",
            "minioadmin",
            "minioadmin",
            "us-east-1",
            Some("http://localhost:9000"),
        )
        .expect("should build with custom endpoint");
        assert_eq!(store.bucket(), "test-bucket");
    }

    #[test]
    fn s3_blob_store_with_prefix() {
        let store = S3BlobStore::from_credentials("b", "AK", "SK", "eu-west-1", None)
            .expect("build")
            .with_prefix("staging/");
        assert_eq!(store.key_prefix(), "staging/");
        assert_eq!(store.full_key("artifacts/a/b/c"), "staging/artifacts/a/b/c");
    }

    #[test]
    fn s3_blob_store_full_key_without_prefix() {
        let store =
            S3BlobStore::from_credentials("b", "AK", "SK", "eu-west-1", None).expect("build");
        assert_eq!(store.full_key("artifacts/a/b/c"), "artifacts/a/b/c");
    }

    #[test]
    fn s3_blob_store_max_bytes_override() {
        let store = S3BlobStore::from_credentials("b", "AK", "SK", "eu-west-1", None)
            .expect("build")
            .max_bytes(1024);
        assert_eq!(store.max_bytes_limit(), 1024);
    }

    #[test]
    fn s3_blob_store_rejects_empty_access_key() {
        let err = S3BlobStore::from_credentials("b", "", "SK", "eu-west-1", None)
            .expect_err("empty access key");
        assert!(matches!(err, ArtifactError::Io(_)));
    }

    #[test]
    fn s3_blob_store_rejects_empty_secret_key() {
        let err = S3BlobStore::from_credentials("b", "AK", "", "eu-west-1", None)
            .expect_err("empty secret key");
        assert!(matches!(err, ArtifactError::Io(_)));
    }

    #[test]
    fn s3_blob_store_debug_does_not_leak_credentials() {
        let store = S3BlobStore::from_credentials(
            "b",
            "AKIAIOSFODNN7EXAMPLE",
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            "eu-west-1",
            None,
        )
        .expect("build");
        let debug = format!("{store:?}");
        assert!(!debug.contains("AKIAIOSFODNN7EXAMPLE"));
        assert!(!debug.contains("wJalrXUtnFEMI"));
    }

    #[tokio::test]
    async fn s3_upload_part_rejects_part_number_beyond_s3_limit() {
        let store =
            S3BlobStore::from_credentials("b", "AK", "SK", "eu-west-1", None).expect("build");
        let mut parts: Vec<CompletedPart> = (1..=MAX_PARTS)
            .map(|n| {
                CompletedPart::builder()
                    .part_number(i32::try_from(n).expect("part number fits"))
                    .e_tag("etag")
                    .build()
            })
            .collect();

        let err = store
            .upload_part("key", "UP1", &mut parts, Bytes::from_static(b"x"))
            .await
            .expect_err("part limit");

        assert!(matches!(&err, ArtifactError::Io(msg) if msg.contains("10000")));
        assert_eq!(parts.len(), MAX_PARTS);
    }
}
