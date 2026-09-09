//! [`AbortMultipartUpload`] operation.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext};
use serde_json::Value;

use crate::S3Client;
use crate::error::sdk_err;

/// Abort an in-progress multipart upload, discarding all uploaded parts.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_s3::{S3Client, multipart::AbortMultipartUpload};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let s3 = S3Client::from_credentials("AKID", "SECRET", "eu-west-1", None)?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = AbortMultipartUpload::new(&s3, "my-bucket", "large-file.bin", "upload-id");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct AbortMultipartUpload {
    client: S3Client,
    bucket: String,
    key: String,
    upload_id: String,
}

impl AbortMultipartUpload {
    /// Create a new abort-multipart-upload operation.
    pub fn new(client: &S3Client, bucket: &str, key: &str, upload_id: &str) -> Self {
        Self {
            client: client.clone(),
            bucket: bucket.to_string(),
            key: key.to_string(),
            upload_id: upload_id.to_string(),
        }
    }

    /// Execute the abort.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on S3 API failure.
    pub async fn run(&self) -> Result<Value, OperationError> {
        self.client
            .client()
            .abort_multipart_upload()
            .bucket(&self.bucket)
            .key(&self.key)
            .upload_id(&self.upload_id)
            .send()
            .await
            .map_err(sdk_err)?;

        Ok(serde_json::json!({}))
    }
}

#[async_trait]
impl Operation for AbortMultipartUpload {
    fn kind(&self) -> &str {
        "s3"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        self.run().await
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "bucket": self.bucket,
            "key": self.key,
            "upload_id": self.upload_id,
        }))
    }
}
