//! Presigned URL operations for temporary access to S3 objects.

use std::time::Duration;

use async_trait::async_trait;
use aws_sdk_s3::presigning::PresigningConfig;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::S3Client;
use crate::error::sdk_err;

/// Output of a presigned URL operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresignedUrlOutput {
    /// The presigned URL.
    pub url: String,
    /// Time-to-live in seconds.
    pub expires_in_secs: u64,
}

/// Generate a presigned GET URL for temporary download access.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_s3::{S3Client, presigned::PresignGetObject};
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let s3 = S3Client::from_credentials("AKID", "SECRET", "eu-west-1", None)?;
/// let output = PresignGetObject::new(&s3, "my-bucket", "file.txt", 3600)
///     .run()
///     .await?;
/// assert!(output.url.contains("X-Amz-Signature"));
/// # Ok(())
/// # }
/// ```
pub struct PresignGetObject {
    client: S3Client,
    bucket: String,
    key: String,
    expires_in_secs: u64,
}

impl PresignGetObject {
    /// Create a new presign-get-object operation.
    pub fn new(client: &S3Client, bucket: &str, key: &str, expires_in_secs: u64) -> Self {
        Self {
            client: client.clone(),
            bucket: bucket.to_string(),
            key: key.to_string(),
            expires_in_secs,
        }
    }

    /// Execute and return the presigned URL.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if presigning fails.
    pub async fn run(&self) -> Result<PresignedUrlOutput, OperationError> {
        let presigning = PresigningConfig::builder()
            .expires_in(Duration::from_secs(self.expires_in_secs))
            .build()
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("presigning config error: {e}"),
            })?;

        let presigned = self
            .client
            .client()
            .get_object()
            .bucket(&self.bucket)
            .key(&self.key)
            .presigned(presigning)
            .await
            .map_err(sdk_err)?;

        Ok(PresignedUrlOutput {
            url: presigned.uri().to_string(),
            expires_in_secs: self.expires_in_secs,
        })
    }
}

#[async_trait]
impl Operation for PresignGetObject {
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
            "expires_in_secs": self.expires_in_secs,
        }))
    }
}

impl TypedOperation for PresignGetObject {
    type Output = PresignedUrlOutput;
}

/// Generate a presigned PUT URL for temporary upload access.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_s3::{S3Client, presigned::PresignPutObject};
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let s3 = S3Client::from_credentials("AKID", "SECRET", "eu-west-1", None)?;
/// let output = PresignPutObject::new(&s3, "my-bucket", "upload.txt", 3600)
///     .with_content_type("text/plain")
///     .run()
///     .await?;
/// assert!(output.url.contains("X-Amz-Signature"));
/// # Ok(())
/// # }
/// ```
pub struct PresignPutObject {
    client: S3Client,
    bucket: String,
    key: String,
    expires_in_secs: u64,
    content_type: Option<String>,
}

impl PresignPutObject {
    /// Create a new presign-put-object operation.
    pub fn new(client: &S3Client, bucket: &str, key: &str, expires_in_secs: u64) -> Self {
        Self {
            client: client.clone(),
            bucket: bucket.to_string(),
            key: key.to_string(),
            expires_in_secs,
            content_type: None,
        }
    }

    /// Set the Content-Type for the presigned upload.
    pub fn with_content_type(mut self, content_type: &str) -> Self {
        self.content_type = Some(content_type.to_string());
        self
    }

    /// Execute and return the presigned URL.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if presigning fails.
    pub async fn run(&self) -> Result<PresignedUrlOutput, OperationError> {
        let presigning = PresigningConfig::builder()
            .expires_in(Duration::from_secs(self.expires_in_secs))
            .build()
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("presigning config error: {e}"),
            })?;

        let mut req = self
            .client
            .client()
            .put_object()
            .bucket(&self.bucket)
            .key(&self.key);

        if let Some(ct) = &self.content_type {
            req = req.content_type(ct);
        }

        let presigned = req.presigned(presigning).await.map_err(sdk_err)?;

        Ok(PresignedUrlOutput {
            url: presigned.uri().to_string(),
            expires_in_secs: self.expires_in_secs,
        })
    }
}

#[async_trait]
impl Operation for PresignPutObject {
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
            "expires_in_secs": self.expires_in_secs,
        }))
    }
}

impl TypedOperation for PresignPutObject {
    type Output = PresignedUrlOutput;
}
