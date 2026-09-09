//! Bucket policy operations.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext};
use serde_json::Value;

use crate::S3Client;
use crate::error::sdk_err;

/// Get the policy of a bucket (as a JSON string).
pub struct GetBucketPolicy {
    client: S3Client,
    bucket: String,
}

impl GetBucketPolicy {
    /// Create a new get-bucket-policy operation.
    pub fn new(client: &S3Client, bucket: &str) -> Self {
        Self {
            client: client.clone(),
            bucket: bucket.to_string(),
        }
    }

    /// Execute and return the bucket policy.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on S3 API failure.
    pub async fn run(&self) -> Result<Value, OperationError> {
        let resp = self
            .client
            .client()
            .get_bucket_policy()
            .bucket(&self.bucket)
            .send()
            .await
            .map_err(sdk_err)?;

        Ok(serde_json::json!({
            "policy": resp.policy(),
        }))
    }
}

#[async_trait]
impl Operation for GetBucketPolicy {
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

/// Set the policy on a bucket.
pub struct PutBucketPolicy {
    client: S3Client,
    bucket: String,
    policy: String,
}

impl PutBucketPolicy {
    /// Create a new put-bucket-policy operation.
    pub fn new(client: &S3Client, bucket: &str, policy: &str) -> Self {
        Self {
            client: client.clone(),
            bucket: bucket.to_string(),
            policy: policy.to_string(),
        }
    }

    /// Execute the policy update.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on S3 API failure.
    pub async fn run(&self) -> Result<Value, OperationError> {
        self.client
            .client()
            .put_bucket_policy()
            .bucket(&self.bucket)
            .policy(&self.policy)
            .send()
            .await
            .map_err(sdk_err)?;

        Ok(serde_json::json!({ "applied": true }))
    }
}

#[async_trait]
impl Operation for PutBucketPolicy {
    fn kind(&self) -> &str {
        "s3"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        self.run().await
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "bucket": self.bucket,
            "policy_length": self.policy.len(),
        }))
    }
}

/// Delete the policy of a bucket.
pub struct DeleteBucketPolicy {
    client: S3Client,
    bucket: String,
}

impl DeleteBucketPolicy {
    /// Create a new delete-bucket-policy operation.
    pub fn new(client: &S3Client, bucket: &str) -> Self {
        Self {
            client: client.clone(),
            bucket: bucket.to_string(),
        }
    }

    /// Execute and delete the bucket policy.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on S3 API failure.
    pub async fn run(&self) -> Result<Value, OperationError> {
        self.client
            .client()
            .delete_bucket_policy()
            .bucket(&self.bucket)
            .send()
            .await
            .map_err(sdk_err)?;

        Ok(serde_json::json!({ "deleted": true }))
    }
}

#[async_trait]
impl Operation for DeleteBucketPolicy {
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
