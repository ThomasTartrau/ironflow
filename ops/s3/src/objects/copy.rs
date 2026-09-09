//! [`CopyObject`] operation.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::S3Client;
use crate::error::sdk_err;

/// Output of a [`CopyObject`] operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CopyObjectOutput {
    /// ETag of the copied object.
    pub etag: Option<String>,
    /// Last modification timestamp (RFC 3339).
    pub last_modified: Option<String>,
    /// Version ID of the copy.
    pub version_id: Option<String>,
}

/// Copy an object within or between buckets.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_s3::{S3Client, objects::CopyObject};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let s3 = S3Client::from_credentials("AKID", "SECRET", "eu-west-1", None)?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = CopyObject::new(&s3, "src-bucket", "src-key", "dst-bucket", "dst-key");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct CopyObject {
    client: S3Client,
    source_bucket: String,
    source_key: String,
    dest_bucket: String,
    dest_key: String,
}

impl CopyObject {
    /// Create a new copy-object operation.
    pub fn new(
        client: &S3Client,
        source_bucket: &str,
        source_key: &str,
        dest_bucket: &str,
        dest_key: &str,
    ) -> Self {
        Self {
            client: client.clone(),
            source_bucket: source_bucket.to_string(),
            source_key: source_key.to_string(),
            dest_bucket: dest_bucket.to_string(),
            dest_key: dest_key.to_string(),
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on S3 API failure.
    pub async fn run(&self) -> Result<CopyObjectOutput, OperationError> {
        let copy_source = format!("{}/{}", self.source_bucket, self.source_key);
        let resp = self
            .client
            .client()
            .copy_object()
            .bucket(&self.dest_bucket)
            .key(&self.dest_key)
            .copy_source(&copy_source)
            .send()
            .await
            .map_err(sdk_err)?;

        let result = resp.copy_object_result();

        Ok(CopyObjectOutput {
            etag: result.and_then(|r| r.e_tag().map(String::from)),
            last_modified: result.and_then(|r| r.last_modified().map(|t| t.to_string())),
            version_id: resp.version_id().map(String::from),
        })
    }
}

#[async_trait]
impl Operation for CopyObject {
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
            "source_bucket": self.source_bucket,
            "source_key": self.source_key,
            "dest_bucket": self.dest_bucket,
            "dest_key": self.dest_key,
        }))
    }
}

impl TypedOperation for CopyObject {
    type Output = CopyObjectOutput;
}
