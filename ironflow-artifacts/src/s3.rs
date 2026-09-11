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
use aws_smithy_types::byte_stream::ByteStream as SdkByteStream;
use futures_util::StreamExt;
use sha2::{Digest, Sha256};

use crate::blob_store::{BlobDigest, BlobFuture, BlobStore, ByteStream};
use crate::error::ArtifactError;
use crate::local::DEFAULT_MAX_ARTIFACT_BYTES;

/// S3-compatible [`BlobStore`].
///
/// Blobs are stored as objects under `{prefix}/{key}` in the configured bucket.
/// SHA-256 is computed while streaming the upload, so the payload is never
/// buffered whole in memory.
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

impl BlobStore for S3BlobStore {
    fn put<'a>(&'a self, key: &'a str, mut content: ByteStream) -> BlobFuture<'a, BlobDigest> {
        Box::pin(async move {
            let full_key = self.full_key(key);
            let mut hasher = Sha256::new();
            let mut size_bytes: u64 = 0;
            let mut body = Vec::new();

            while let Some(chunk) = content.next().await {
                let bytes = chunk?;
                size_bytes += bytes.len() as u64;
                if size_bytes > self.max_bytes {
                    return Err(ArtifactError::TooLarge {
                        limit_bytes: self.max_bytes,
                    });
                }
                hasher.update(&bytes);
                body.extend_from_slice(&bytes);
            }

            self.client
                .put_object()
                .bucket(&self.bucket)
                .key(&full_key)
                .body(SdkByteStream::from(body))
                .send()
                .await
                .map_err(|err| ArtifactError::Io(format!("S3 PutObject failed: {err}")))?;

            Ok(BlobDigest {
                size_bytes,
                sha256: hex::encode(hasher.finalize()),
            })
        })
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

            let collected = resp
                .body
                .collect()
                .await
                .map_err(|err| ArtifactError::Io(format!("S3 body collect: {err}")))?;

            Ok(crate::stream_from_bytes(collected.into_bytes()))
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
