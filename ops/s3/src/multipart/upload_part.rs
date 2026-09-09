//! [`UploadPart`] operation.

use async_trait::async_trait;
use aws_sdk_s3::primitives::ByteStream;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::S3Client;
use crate::error::sdk_err;

/// Output of an [`UploadPart`] operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadPartOutput {
    /// ETag of the uploaded part (needed for completion).
    pub etag: Option<String>,
}

/// Upload a single part of a multipart upload.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_s3::{S3Client, multipart::UploadPart};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let s3 = S3Client::from_credentials("AKID", "SECRET", "eu-west-1", None)?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = UploadPart::new(&s3, "bucket", "key", "upload-id", 1, vec![0u8; 1024]);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct UploadPart {
    client: S3Client,
    bucket: String,
    key: String,
    upload_id: String,
    part_number: i32,
    body: Vec<u8>,
}

impl UploadPart {
    /// Create a new upload-part operation.
    pub fn new(
        client: &S3Client,
        bucket: &str,
        key: &str,
        upload_id: &str,
        part_number: i32,
        body: Vec<u8>,
    ) -> Self {
        Self {
            client: client.clone(),
            bucket: bucket.to_string(),
            key: key.to_string(),
            upload_id: upload_id.to_string(),
            part_number,
            body,
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on S3 API failure.
    pub async fn run(&self) -> Result<UploadPartOutput, OperationError> {
        let resp = self
            .client
            .client()
            .upload_part()
            .bucket(&self.bucket)
            .key(&self.key)
            .upload_id(&self.upload_id)
            .part_number(self.part_number)
            .body(ByteStream::from(self.body.clone()))
            .send()
            .await
            .map_err(sdk_err)?;

        Ok(UploadPartOutput {
            etag: resp.e_tag().map(String::from),
        })
    }
}

#[async_trait]
impl Operation for UploadPart {
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
            "upload_id": self.upload_id,
            "part_number": self.part_number,
            "body_size": self.body.len(),
        }))
    }
}

impl TypedOperation for UploadPart {
    type Output = UploadPartOutput;
}
