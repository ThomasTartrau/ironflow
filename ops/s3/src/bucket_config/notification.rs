//! Bucket notification operations.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext};
use serde_json::Value;

use crate::S3Client;
use crate::error::sdk_err;

/// Get the notification configuration of a bucket.
pub struct GetBucketNotification {
    client: S3Client,
    bucket: String,
}

impl GetBucketNotification {
    /// Create a new get-bucket-notification operation.
    pub fn new(client: &S3Client, bucket: &str) -> Self {
        Self {
            client: client.clone(),
            bucket: bucket.to_string(),
        }
    }

    /// Execute and return notification configuration summary.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on S3 API failure.
    pub async fn run(&self) -> Result<Value, OperationError> {
        let resp = self
            .client
            .client()
            .get_bucket_notification_configuration()
            .bucket(&self.bucket)
            .send()
            .await
            .map_err(sdk_err)?;

        Ok(serde_json::json!({
            "topic_configurations": resp.topic_configurations().len(),
            "queue_configurations": resp.queue_configurations().len(),
            "lambda_function_configurations": resp.lambda_function_configurations().len(),
        }))
    }
}

#[async_trait]
impl Operation for GetBucketNotification {
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
