//! [`GetObject`] operation.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::S3Client;
use crate::error::sdk_err;

/// Output of a [`GetObject`] operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetObjectOutput {
    /// Content-Type of the object.
    pub content_type: Option<String>,
    /// Size of the object in bytes.
    pub content_length: Option<i64>,
    /// ETag of the object.
    pub etag: Option<String>,
    /// Last modification timestamp (RFC 3339).
    pub last_modified: Option<String>,
    /// Version ID (if bucket versioning is enabled).
    pub version_id: Option<String>,
    /// The object body as raw bytes (not serialized to JSON).
    #[serde(skip)]
    pub body: Vec<u8>,
}

/// Download an object from S3.
///
/// The [`run`](GetObject::run) method returns the full body as bytes in the
/// output struct. The [`Operation::execute`] implementation returns metadata
/// only (no body) to avoid bloating JSON step output with large binary data.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_s3::{S3Client, objects::GetObject};
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let s3 = S3Client::from_credentials("AKID", "SECRET", "eu-west-1", None)?;
/// let output = GetObject::new(&s3, "my-bucket", "path/to/file.txt").run().await?;
/// assert!(!output.body.is_empty());
/// # Ok(())
/// # }
/// ```
pub struct GetObject {
    client: S3Client,
    bucket: String,
    key: String,
}

impl GetObject {
    /// Create a new get-object operation.
    pub fn new(client: &S3Client, bucket: &str, key: &str) -> Self {
        Self {
            client: client.clone(),
            bucket: bucket.to_string(),
            key: key.to_string(),
        }
    }

    /// Execute and return metadata with the full body bytes.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on S3 API failure.
    pub async fn run(&self) -> Result<GetObjectOutput, OperationError> {
        let resp = self
            .client
            .client()
            .get_object()
            .bucket(&self.bucket)
            .key(&self.key)
            .send()
            .await
            .map_err(sdk_err)?;

        let content_type = resp.content_type().map(String::from);
        let content_length = resp.content_length();
        let etag = resp.e_tag().map(String::from);
        let last_modified = resp.last_modified().map(|t| t.to_string());
        let version_id = resp.version_id().map(String::from);

        let body = resp
            .body
            .collect()
            .await
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("failed to read S3 response body: {e}"),
            })?
            .into_bytes()
            .to_vec();

        Ok(GetObjectOutput {
            content_type,
            content_length,
            etag,
            last_modified,
            version_id,
            body,
        })
    }
}

#[async_trait]
impl Operation for GetObject {
    fn kind(&self) -> &str {
        "s3"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let output = self.run().await?;
        Ok(serde_json::json!({
            "content_type": output.content_type,
            "content_length": output.content_length,
            "etag": output.etag,
            "last_modified": output.last_modified,
            "version_id": output.version_id,
            "body_size": output.body.len(),
        }))
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "bucket": self.bucket,
            "key": self.key,
        }))
    }
}

impl TypedOperation for GetObject {
    type Output = GetObjectOutput;
}
