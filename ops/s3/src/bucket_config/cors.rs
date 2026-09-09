//! Bucket CORS operations.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext};
use serde_json::Value;

use crate::S3Client;
use crate::error::sdk_err;

/// Get the CORS configuration of a bucket.
pub struct GetBucketCors {
    client: S3Client,
    bucket: String,
}

impl GetBucketCors {
    /// Create a new get-bucket-cors operation.
    pub fn new(client: &S3Client, bucket: &str) -> Self {
        Self {
            client: client.clone(),
            bucket: bucket.to_string(),
        }
    }

    /// Execute and return the CORS rules.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on S3 API failure.
    pub async fn run(&self) -> Result<Value, OperationError> {
        let resp = self
            .client
            .client()
            .get_bucket_cors()
            .bucket(&self.bucket)
            .send()
            .await
            .map_err(sdk_err)?;

        let rules: Vec<Value> = resp
            .cors_rules()
            .iter()
            .map(|r| {
                serde_json::json!({
                    "allowed_origins": r.allowed_origins(),
                    "allowed_methods": r.allowed_methods(),
                    "allowed_headers": r.allowed_headers(),
                    "max_age_seconds": r.max_age_seconds(),
                })
            })
            .collect();

        Ok(serde_json::json!({
            "rule_count": rules.len(),
            "rules": rules,
        }))
    }
}

#[async_trait]
impl Operation for GetBucketCors {
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

/// Delete the CORS configuration of a bucket.
pub struct DeleteBucketCors {
    client: S3Client,
    bucket: String,
}

impl DeleteBucketCors {
    /// Create a new delete-bucket-cors operation.
    pub fn new(client: &S3Client, bucket: &str) -> Self {
        Self {
            client: client.clone(),
            bucket: bucket.to_string(),
        }
    }

    /// Execute and delete the CORS configuration.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on S3 API failure.
    pub async fn run(&self) -> Result<Value, OperationError> {
        self.client
            .client()
            .delete_bucket_cors()
            .bucket(&self.bucket)
            .send()
            .await
            .map_err(sdk_err)?;

        Ok(serde_json::json!({ "deleted": true }))
    }
}

#[async_trait]
impl Operation for DeleteBucketCors {
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
