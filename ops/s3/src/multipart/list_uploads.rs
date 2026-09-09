//! [`ListMultipartUploads`] operation.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::S3Client;
use crate::error::sdk_err;

/// Information about an in-progress multipart upload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultipartUploadInfo {
    /// Object key.
    pub key: Option<String>,
    /// Upload ID.
    pub upload_id: Option<String>,
    /// Timestamp when the upload was initiated (RFC 3339).
    pub initiated: Option<String>,
}

/// Output of a [`ListMultipartUploads`] operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListMultipartUploadsOutput {
    /// Active multipart uploads.
    pub uploads: Vec<MultipartUploadInfo>,
}

/// List in-progress multipart uploads for a bucket.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_s3::{S3Client, multipart::ListMultipartUploads};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let s3 = S3Client::from_credentials("AKID", "SECRET", "eu-west-1", None)?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = ListMultipartUploads::new(&s3, "my-bucket");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct ListMultipartUploads {
    client: S3Client,
    bucket: String,
    prefix: Option<String>,
}

impl ListMultipartUploads {
    /// Create a new list-multipart-uploads operation.
    pub fn new(client: &S3Client, bucket: &str) -> Self {
        Self {
            client: client.clone(),
            bucket: bucket.to_string(),
            prefix: None,
        }
    }

    /// Filter results to keys starting with this prefix.
    pub fn with_prefix(mut self, prefix: &str) -> Self {
        self.prefix = Some(prefix.to_string());
        self
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on S3 API failure.
    pub async fn run(&self) -> Result<ListMultipartUploadsOutput, OperationError> {
        let mut req = self
            .client
            .client()
            .list_multipart_uploads()
            .bucket(&self.bucket);

        if let Some(prefix) = &self.prefix {
            req = req.prefix(prefix);
        }

        let resp = req.send().await.map_err(sdk_err)?;

        let uploads = resp
            .uploads()
            .iter()
            .map(|u| MultipartUploadInfo {
                key: u.key().map(String::from),
                upload_id: u.upload_id().map(String::from),
                initiated: u.initiated().map(|t| t.to_string()),
            })
            .collect();

        Ok(ListMultipartUploadsOutput { uploads })
    }
}

#[async_trait]
impl Operation for ListMultipartUploads {
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
            "prefix": self.prefix,
        }))
    }
}

impl TypedOperation for ListMultipartUploads {
    type Output = ListMultipartUploadsOutput;
}
