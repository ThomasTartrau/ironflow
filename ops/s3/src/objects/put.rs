//! [`PutObject`] operation.

use async_trait::async_trait;
use aws_sdk_s3::primitives::ByteStream;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::S3Client;
use crate::error::sdk_err;

/// Output of a [`PutObject`] operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PutObjectOutput {
    /// ETag of the uploaded object.
    pub etag: Option<String>,
    /// Version ID (if bucket versioning is enabled).
    pub version_id: Option<String>,
}

/// Upload an object to S3.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_s3::{S3Client, objects::PutObject};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let s3 = S3Client::from_credentials("AKID", "SECRET", "eu-west-1", None)?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = PutObject::new(&s3, "my-bucket", "path/to/file.txt", b"hello world".to_vec());
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct PutObject {
    client: S3Client,
    bucket: String,
    key: String,
    body: Vec<u8>,
    content_type: Option<String>,
}

impl PutObject {
    /// Create a new put-object operation.
    pub fn new(client: &S3Client, bucket: &str, key: &str, body: Vec<u8>) -> Self {
        Self {
            client: client.clone(),
            bucket: bucket.to_string(),
            key: key.to_string(),
            body,
            content_type: None,
        }
    }

    /// Set the Content-Type header for the upload.
    pub fn with_content_type(mut self, content_type: &str) -> Self {
        self.content_type = Some(content_type.to_string());
        self
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on S3 API failure.
    pub async fn run(&self) -> Result<PutObjectOutput, OperationError> {
        let mut req = self
            .client
            .client()
            .put_object()
            .bucket(&self.bucket)
            .key(&self.key)
            .body(ByteStream::from(self.body.clone()));

        if let Some(ct) = &self.content_type {
            req = req.content_type(ct);
        }

        let resp = req.send().await.map_err(sdk_err)?;

        Ok(PutObjectOutput {
            etag: resp.e_tag().map(String::from),
            version_id: resp.version_id().map(String::from),
        })
    }
}

#[async_trait]
impl Operation for PutObject {
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
            "body_size": self.body.len(),
        }))
    }
}

impl TypedOperation for PutObject {
    type Output = PutObjectOutput;
}
