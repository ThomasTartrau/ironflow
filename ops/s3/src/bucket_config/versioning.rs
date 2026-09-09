//! Bucket versioning operations.

use async_trait::async_trait;
use aws_sdk_s3::types::{BucketVersioningStatus, MfaDeleteStatus, VersioningConfiguration};
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext};
use serde_json::Value;

use crate::S3Client;
use crate::error::sdk_err;

/// Get the versioning state of a bucket.
pub struct GetBucketVersioning {
    client: S3Client,
    bucket: String,
}

impl GetBucketVersioning {
    /// Create a new get-bucket-versioning operation.
    pub fn new(client: &S3Client, bucket: &str) -> Self {
        Self {
            client: client.clone(),
            bucket: bucket.to_string(),
        }
    }

    /// Execute and return the versioning status.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on S3 API failure.
    pub async fn run(&self) -> Result<Value, OperationError> {
        let resp = self
            .client
            .client()
            .get_bucket_versioning()
            .bucket(&self.bucket)
            .send()
            .await
            .map_err(sdk_err)?;

        Ok(serde_json::json!({
            "status": resp.status().map(BucketVersioningStatus::as_str),
            "mfa_delete": resp.mfa_delete().map(MfaDeleteStatus::as_str),
        }))
    }
}

#[async_trait]
impl Operation for GetBucketVersioning {
    fn kind(&self) -> &str {
        "s3"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        self.run().await
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "bucket": self.bucket }))
    }
}

/// Enable or suspend versioning on a bucket.
///
/// Pass `"Enabled"` or `"Suspended"` as the status.
pub struct PutBucketVersioning {
    client: S3Client,
    bucket: String,
    status: String,
}

impl PutBucketVersioning {
    /// Create a new put-bucket-versioning operation.
    pub fn new(client: &S3Client, bucket: &str, status: &str) -> Self {
        Self {
            client: client.clone(),
            bucket: bucket.to_string(),
            status: status.to_string(),
        }
    }

    /// Execute the versioning configuration change.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on S3 API failure.
    pub async fn run(&self) -> Result<Value, OperationError> {
        let versioning_status = match self.status.as_str() {
            "Enabled" => BucketVersioningStatus::Enabled,
            "Suspended" => BucketVersioningStatus::Suspended,
            other => {
                return Err(OperationError::Http {
                    status: None,
                    message: format!(
                        "invalid versioning status '{other}': expected 'Enabled' or 'Suspended'"
                    ),
                });
            }
        };

        let config = VersioningConfiguration::builder()
            .status(versioning_status)
            .build();

        self.client
            .client()
            .put_bucket_versioning()
            .bucket(&self.bucket)
            .versioning_configuration(config)
            .send()
            .await
            .map_err(sdk_err)?;

        Ok(serde_json::json!({ "status": self.status }))
    }
}

#[async_trait]
impl Operation for PutBucketVersioning {
    fn kind(&self) -> &str {
        "s3"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        self.run().await
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "bucket": self.bucket,
            "status": self.status,
        }))
    }
}
