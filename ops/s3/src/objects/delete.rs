//! [`DeleteObject`] and [`DeleteObjects`] operations.

use async_trait::async_trait;
use aws_sdk_s3::types::{Delete, ObjectIdentifier};
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::S3Client;
use crate::error::sdk_err;

/// Delete a single object from S3.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_s3::{S3Client, objects::DeleteObject};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let s3 = S3Client::from_credentials("AKID", "SECRET", "eu-west-1", None)?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = DeleteObject::new(&s3, "my-bucket", "path/to/file.txt");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct DeleteObject {
    client: S3Client,
    bucket: String,
    key: String,
}

impl DeleteObject {
    /// Create a new delete-object operation.
    pub fn new(client: &S3Client, bucket: &str, key: &str) -> Self {
        Self {
            client: client.clone(),
            bucket: bucket.to_string(),
            key: key.to_string(),
        }
    }

    /// Execute and return the raw JSON response.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on S3 API failure.
    pub async fn run(&self) -> Result<Value, OperationError> {
        let resp = self
            .client
            .client()
            .delete_object()
            .bucket(&self.bucket)
            .key(&self.key)
            .send()
            .await
            .map_err(sdk_err)?;

        Ok(serde_json::json!({
            "version_id": resp.version_id(),
            "delete_marker": resp.delete_marker(),
        }))
    }
}

#[async_trait]
impl Operation for DeleteObject {
    fn kind(&self) -> &str {
        "s3"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        self.run().await
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "bucket": self.bucket,
            "key": self.key,
        }))
    }
}

/// Output of a [`DeleteObjects`] operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteObjectsOutput {
    /// Keys that were successfully deleted.
    pub deleted: Vec<String>,
    /// Keys that failed to delete, with error details.
    pub errors: Vec<DeleteObjectError>,
}

/// Error detail for a single failed deletion in a batch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteObjectError {
    /// Object key.
    pub key: Option<String>,
    /// Error code.
    pub code: Option<String>,
    /// Error message.
    pub message: Option<String>,
}

/// Delete multiple objects in a single request (up to 1000 keys).
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_s3::{S3Client, objects::DeleteObjects};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let s3 = S3Client::from_credentials("AKID", "SECRET", "eu-west-1", None)?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let keys = vec!["file1.txt".to_string(), "file2.txt".to_string()];
/// let op = DeleteObjects::new(&s3, "my-bucket", keys);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct DeleteObjects {
    client: S3Client,
    bucket: String,
    keys: Vec<String>,
}

impl DeleteObjects {
    /// Create a new batch delete operation.
    pub fn new(client: &S3Client, bucket: &str, keys: Vec<String>) -> Self {
        Self {
            client: client.clone(),
            bucket: bucket.to_string(),
            keys,
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on S3 API failure.
    pub async fn run(&self) -> Result<DeleteObjectsOutput, OperationError> {
        let objects = self
            .keys
            .iter()
            .map(|k| {
                ObjectIdentifier::builder()
                    .key(k)
                    .build()
                    .map_err(|e| OperationError::Http {
                        status: None,
                        message: format!("failed to build object identifier: {e}"),
                    })
            })
            .collect::<Result<Vec<ObjectIdentifier>, OperationError>>()?;

        let delete = Delete::builder()
            .set_objects(Some(objects))
            .build()
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("failed to build delete request: {e}"),
            })?;

        let resp = self
            .client
            .client()
            .delete_objects()
            .bucket(&self.bucket)
            .delete(delete)
            .send()
            .await
            .map_err(sdk_err)?;

        let deleted = resp
            .deleted()
            .iter()
            .filter_map(|d| d.key().map(String::from))
            .collect();

        let errors = resp
            .errors()
            .iter()
            .map(|e| DeleteObjectError {
                key: e.key().map(String::from),
                code: e.code().map(String::from),
                message: e.message().map(String::from),
            })
            .collect();

        Ok(DeleteObjectsOutput { deleted, errors })
    }
}

#[async_trait]
impl Operation for DeleteObjects {
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
            "key_count": self.keys.len(),
        }))
    }
}

impl TypedOperation for DeleteObjects {
    type Output = DeleteObjectsOutput;
}
