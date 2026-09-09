//! [`ListParts`] operation.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::S3Client;
use crate::error::sdk_err;

/// Information about a single uploaded part.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartInfo {
    /// Part number.
    pub part_number: Option<i32>,
    /// Size in bytes.
    pub size: Option<i64>,
    /// ETag of the part.
    pub etag: Option<String>,
}

/// Output of a [`ListParts`] operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListPartsOutput {
    /// Uploaded parts.
    pub parts: Vec<PartInfo>,
}

/// List the parts that have been uploaded for a multipart upload.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_s3::{S3Client, multipart::ListParts};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let s3 = S3Client::from_credentials("AKID", "SECRET", "eu-west-1", None)?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = ListParts::new(&s3, "my-bucket", "large-file.bin", "upload-id");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct ListParts {
    client: S3Client,
    bucket: String,
    key: String,
    upload_id: String,
}

impl ListParts {
    /// Create a new list-parts operation.
    pub fn new(client: &S3Client, bucket: &str, key: &str, upload_id: &str) -> Self {
        Self {
            client: client.clone(),
            bucket: bucket.to_string(),
            key: key.to_string(),
            upload_id: upload_id.to_string(),
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on S3 API failure.
    pub async fn run(&self) -> Result<ListPartsOutput, OperationError> {
        let resp = self
            .client
            .client()
            .list_parts()
            .bucket(&self.bucket)
            .key(&self.key)
            .upload_id(&self.upload_id)
            .send()
            .await
            .map_err(sdk_err)?;

        let parts = resp
            .parts()
            .iter()
            .map(|p| PartInfo {
                part_number: p.part_number(),
                size: p.size(),
                etag: p.e_tag().map(String::from),
            })
            .collect();

        Ok(ListPartsOutput { parts })
    }
}

#[async_trait]
impl Operation for ListParts {
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
        }))
    }
}

impl TypedOperation for ListParts {
    type Output = ListPartsOutput;
}
