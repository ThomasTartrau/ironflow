//! Bucket lifecycle operations.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext};
use serde_json::Value;

use crate::S3Client;
use crate::error::sdk_err;

/// Get the lifecycle configuration of a bucket.
pub struct GetBucketLifecycle {
    client: S3Client,
    bucket: String,
}

impl GetBucketLifecycle {
    /// Create a new get-bucket-lifecycle operation.
    pub fn new(client: &S3Client, bucket: &str) -> Self {
        Self {
            client: client.clone(),
            bucket: bucket.to_string(),
        }
    }

    /// Execute and return lifecycle rule count.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on S3 API failure.
    pub async fn run(&self) -> Result<Value, OperationError> {
        let resp = self
            .client
            .client()
            .get_bucket_lifecycle_configuration()
            .bucket(&self.bucket)
            .send()
            .await
            .map_err(sdk_err)?;

        let rules: Vec<Value> = resp
            .rules()
            .iter()
            .map(|r| {
                serde_json::json!({
                    "id": r.id(),
                    "status": r.status().as_str(),
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
impl Operation for GetBucketLifecycle {
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

/// Delete the lifecycle configuration of a bucket.
pub struct DeleteBucketLifecycle {
    client: S3Client,
    bucket: String,
}

impl DeleteBucketLifecycle {
    /// Create a new delete-bucket-lifecycle operation.
    pub fn new(client: &S3Client, bucket: &str) -> Self {
        Self {
            client: client.clone(),
            bucket: bucket.to_string(),
        }
    }

    /// Execute and delete the lifecycle configuration.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on S3 API failure.
    pub async fn run(&self) -> Result<Value, OperationError> {
        self.client
            .client()
            .delete_bucket_lifecycle()
            .bucket(&self.bucket)
            .send()
            .await
            .map_err(sdk_err)?;

        Ok(serde_json::json!({ "deleted": true }))
    }
}

#[async_trait]
impl Operation for DeleteBucketLifecycle {
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
