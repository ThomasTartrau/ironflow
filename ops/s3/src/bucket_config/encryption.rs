//! Bucket encryption operations.

use async_trait::async_trait;
use aws_sdk_s3::types::{
    ServerSideEncryptionByDefault, ServerSideEncryptionConfiguration, ServerSideEncryptionRule,
};
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext};
use serde_json::Value;

use crate::S3Client;
use crate::error::sdk_err;

/// Get the default encryption configuration of a bucket.
pub struct GetBucketEncryption {
    client: S3Client,
    bucket: String,
}

impl GetBucketEncryption {
    /// Create a new get-bucket-encryption operation.
    pub fn new(client: &S3Client, bucket: &str) -> Self {
        Self {
            client: client.clone(),
            bucket: bucket.to_string(),
        }
    }

    /// Execute and return the encryption configuration.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on S3 API failure.
    pub async fn run(&self) -> Result<Value, OperationError> {
        let resp = self
            .client
            .client()
            .get_bucket_encryption()
            .bucket(&self.bucket)
            .send()
            .await
            .map_err(sdk_err)?;

        let rules: Vec<Value> = resp
            .server_side_encryption_configuration()
            .map(|c| {
                c.rules()
                    .iter()
                    .map(|r| {
                        let default = r.apply_server_side_encryption_by_default();
                        serde_json::json!({
                            "sse_algorithm": default.map(|d| d.sse_algorithm().as_str()),
                            "kms_master_key_id": default.and_then(|d| d.kms_master_key_id()),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(serde_json::json!({
            "rule_count": rules.len(),
            "rules": rules,
        }))
    }
}

#[async_trait]
impl Operation for GetBucketEncryption {
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

/// Set the default encryption on a bucket using a specified SSE algorithm.
///
/// Pass `"AES256"` for S3-managed keys or `"aws:kms"` for KMS.
pub struct PutBucketEncryption {
    client: S3Client,
    bucket: String,
    sse_algorithm: String,
}

impl PutBucketEncryption {
    /// Create a new put-bucket-encryption operation.
    pub fn new(client: &S3Client, bucket: &str, sse_algorithm: &str) -> Self {
        Self {
            client: client.clone(),
            bucket: bucket.to_string(),
            sse_algorithm: sse_algorithm.to_string(),
        }
    }

    /// Execute the encryption configuration change.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on S3 API failure or invalid algorithm.
    pub async fn run(&self) -> Result<Value, OperationError> {
        let algorithm = self
            .sse_algorithm
            .as_str()
            .parse()
            .map_err(|_| OperationError::Http {
                status: None,
                message: format!(
                    "invalid SSE algorithm '{}': expected 'AES256' or 'aws:kms'",
                    self.sse_algorithm
                ),
            })?;

        let sse_default = ServerSideEncryptionByDefault::builder()
            .sse_algorithm(algorithm)
            .build()
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("failed to build SSE default: {e}"),
            })?;

        let rule = ServerSideEncryptionRule::builder()
            .apply_server_side_encryption_by_default(sse_default)
            .build();

        let config = ServerSideEncryptionConfiguration::builder()
            .rules(rule)
            .build()
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("failed to build encryption config: {e}"),
            })?;

        self.client
            .client()
            .put_bucket_encryption()
            .bucket(&self.bucket)
            .server_side_encryption_configuration(config)
            .send()
            .await
            .map_err(sdk_err)?;

        Ok(serde_json::json!({
            "sse_algorithm": self.sse_algorithm,
            "applied": true,
        }))
    }
}

#[async_trait]
impl Operation for PutBucketEncryption {
    fn kind(&self) -> &str {
        "s3"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        self.run().await
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "bucket": self.bucket,
            "sse_algorithm": self.sse_algorithm,
        }))
    }
}
