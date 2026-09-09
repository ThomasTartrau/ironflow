//! [`HeadObject`] operation.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::S3Client;
use crate::error::sdk_err;

/// Output of a [`HeadObject`] operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeadObjectOutput {
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
}

/// Retrieve object metadata without downloading the body.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_s3::{S3Client, objects::HeadObject};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let s3 = S3Client::from_credentials("AKID", "SECRET", "eu-west-1", None)?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = HeadObject::new(&s3, "my-bucket", "path/to/file.txt");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct HeadObject {
    client: S3Client,
    bucket: String,
    key: String,
}

impl HeadObject {
    /// Create a new head-object operation.
    pub fn new(client: &S3Client, bucket: &str, key: &str) -> Self {
        Self {
            client: client.clone(),
            bucket: bucket.to_string(),
            key: key.to_string(),
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on S3 API failure (including 404 for
    /// non-existent keys).
    pub async fn run(&self) -> Result<HeadObjectOutput, OperationError> {
        let resp = self
            .client
            .client()
            .head_object()
            .bucket(&self.bucket)
            .key(&self.key)
            .send()
            .await
            .map_err(sdk_err)?;

        Ok(HeadObjectOutput {
            content_type: resp.content_type().map(String::from),
            content_length: resp.content_length(),
            etag: resp.e_tag().map(String::from),
            last_modified: resp.last_modified().map(|t| t.to_string()),
            version_id: resp.version_id().map(String::from),
        })
    }
}

#[async_trait]
impl Operation for HeadObject {
    fn kind(&self) -> &str {
        "s3"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let output = self.run().await?;
        serde_json::to_value(&output).map_err(|e| OperationError::Http {
            status: None,
            message: format!("serialization error: {e}"),
        })
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "bucket": self.bucket,
            "key": self.key,
        }))
    }
}

impl TypedOperation for HeadObject {
    type Output = HeadObjectOutput;
}
