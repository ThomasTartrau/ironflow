//! [`CreateMultipartUpload`] operation.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::S3Client;
use crate::error::sdk_err;

/// Output of a [`CreateMultipartUpload`] operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateMultipartUploadOutput {
    /// Upload ID to reference in subsequent part uploads.
    pub upload_id: Option<String>,
    /// Bucket name.
    pub bucket: Option<String>,
    /// Object key.
    pub key: Option<String>,
}

/// Initiate a multipart upload and obtain an upload ID.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_s3::{S3Client, multipart::CreateMultipartUpload};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let s3 = S3Client::from_credentials("AKID", "SECRET", "eu-west-1", None)?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = CreateMultipartUpload::new(&s3, "my-bucket", "large-file.bin");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct CreateMultipartUpload {
    client: S3Client,
    bucket: String,
    key: String,
    content_type: Option<String>,
}

impl CreateMultipartUpload {
    /// Create a new create-multipart-upload operation.
    pub fn new(client: &S3Client, bucket: &str, key: &str) -> Self {
        Self {
            client: client.clone(),
            bucket: bucket.to_string(),
            key: key.to_string(),
            content_type: None,
        }
    }

    /// Set the Content-Type for the resulting object.
    pub fn with_content_type(mut self, content_type: &str) -> Self {
        self.content_type = Some(content_type.to_string());
        self
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on S3 API failure.
    pub async fn run(&self) -> Result<CreateMultipartUploadOutput, OperationError> {
        let mut req = self
            .client
            .client()
            .create_multipart_upload()
            .bucket(&self.bucket)
            .key(&self.key);

        if let Some(ct) = &self.content_type {
            req = req.content_type(ct);
        }

        let resp = req.send().await.map_err(sdk_err)?;

        Ok(CreateMultipartUploadOutput {
            upload_id: resp.upload_id().map(String::from),
            bucket: resp.bucket().map(String::from),
            key: resp.key().map(String::from),
        })
    }
}

#[async_trait]
impl Operation for CreateMultipartUpload {
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

impl TypedOperation for CreateMultipartUpload {
    type Output = CreateMultipartUploadOutput;
}
