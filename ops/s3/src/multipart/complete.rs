//! [`CompleteMultipartUpload`] operation.

use async_trait::async_trait;
use aws_sdk_s3::types::{CompletedMultipartUpload, CompletedPart};
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::S3Client;
use crate::error::sdk_err;

/// A completed part reference for [`CompleteMultipartUpload`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletedPartInput {
    /// Part number (1-based).
    pub part_number: i32,
    /// ETag returned by [`UploadPart`](super::UploadPart).
    pub etag: String,
}

/// Output of a [`CompleteMultipartUpload`] operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompleteMultipartUploadOutput {
    /// URL of the completed object.
    pub location: Option<String>,
    /// ETag of the assembled object.
    pub etag: Option<String>,
    /// Version ID (if bucket versioning is enabled).
    pub version_id: Option<String>,
}

/// Finish a multipart upload by assembling previously uploaded parts.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_s3::{S3Client, multipart::{CompleteMultipartUpload, CompletedPartInput}};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let s3 = S3Client::from_credentials("AKID", "SECRET", "eu-west-1", None)?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let parts = vec![CompletedPartInput { part_number: 1, etag: "\"abc\"".into() }];
/// let op = CompleteMultipartUpload::new(&s3, "bucket", "key", "upload-id", parts);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct CompleteMultipartUpload {
    client: S3Client,
    bucket: String,
    key: String,
    upload_id: String,
    parts: Vec<CompletedPartInput>,
}

impl CompleteMultipartUpload {
    /// Create a new complete-multipart-upload operation.
    pub fn new(
        client: &S3Client,
        bucket: &str,
        key: &str,
        upload_id: &str,
        parts: Vec<CompletedPartInput>,
    ) -> Self {
        Self {
            client: client.clone(),
            bucket: bucket.to_string(),
            key: key.to_string(),
            upload_id: upload_id.to_string(),
            parts,
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on S3 API failure.
    pub async fn run(&self) -> Result<CompleteMultipartUploadOutput, OperationError> {
        let completed_parts: Vec<CompletedPart> = self
            .parts
            .iter()
            .map(|p| {
                CompletedPart::builder()
                    .part_number(p.part_number)
                    .e_tag(&p.etag)
                    .build()
            })
            .collect();

        let upload = CompletedMultipartUpload::builder()
            .set_parts(Some(completed_parts))
            .build();

        let resp = self
            .client
            .client()
            .complete_multipart_upload()
            .bucket(&self.bucket)
            .key(&self.key)
            .upload_id(&self.upload_id)
            .multipart_upload(upload)
            .send()
            .await
            .map_err(sdk_err)?;

        Ok(CompleteMultipartUploadOutput {
            location: resp.location().map(String::from),
            etag: resp.e_tag().map(String::from),
            version_id: resp.version_id().map(String::from),
        })
    }
}

#[async_trait]
impl Operation for CompleteMultipartUpload {
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
            "part_count": self.parts.len(),
        }))
    }
}

impl TypedOperation for CompleteMultipartUpload {
    type Output = CompleteMultipartUploadOutput;
}
