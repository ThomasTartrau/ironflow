//! Object tagging operations: get, put, and delete tags on S3 objects.

use async_trait::async_trait;
use aws_sdk_s3::types::{Tag as SdkTag, Tagging};
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::S3Client;
use crate::error::sdk_err;

/// A single key-value tag.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tag {
    /// Tag key.
    pub key: String,
    /// Tag value.
    pub value: String,
}

/// Output of a [`GetObjectTagging`] operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetObjectTaggingOutput {
    /// The tags attached to the object.
    pub tags: Vec<Tag>,
}

/// Retrieve the tags attached to an S3 object.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_s3::{S3Client, tagging::GetObjectTagging};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let s3 = S3Client::from_credentials("AKID", "SECRET", "eu-west-1", None)?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = GetObjectTagging::new(&s3, "my-bucket", "path/to/file.txt");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct GetObjectTagging {
    client: S3Client,
    bucket: String,
    key: String,
}

impl GetObjectTagging {
    /// Create a new get-object-tagging operation.
    pub fn new(client: &S3Client, bucket: &str, key: &str) -> Self {
        Self {
            client: client.clone(),
            bucket: bucket.to_string(),
            key: key.to_string(),
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on S3 API failure.
    pub async fn run(&self) -> Result<GetObjectTaggingOutput, OperationError> {
        let resp = self
            .client
            .client()
            .get_object_tagging()
            .bucket(&self.bucket)
            .key(&self.key)
            .send()
            .await
            .map_err(sdk_err)?;

        let tags = resp
            .tag_set()
            .iter()
            .map(|t| Tag {
                key: t.key().to_string(),
                value: t.value().to_string(),
            })
            .collect();

        Ok(GetObjectTaggingOutput { tags })
    }
}

#[async_trait]
impl Operation for GetObjectTagging {
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
        }))
    }
}

impl TypedOperation for GetObjectTagging {
    type Output = GetObjectTaggingOutput;
}

/// Set tags on an S3 object, replacing any existing tags.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_s3::{S3Client, tagging::PutObjectTagging};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let s3 = S3Client::from_credentials("AKID", "SECRET", "eu-west-1", None)?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let tags = vec![("env".to_string(), "prod".to_string())];
/// let op = PutObjectTagging::new(&s3, "my-bucket", "path/to/file.txt", tags);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct PutObjectTagging {
    client: S3Client,
    bucket: String,
    key: String,
    tags: Vec<(String, String)>,
}

impl PutObjectTagging {
    /// Create a new put-object-tagging operation.
    pub fn new(client: &S3Client, bucket: &str, key: &str, tags: Vec<(String, String)>) -> Self {
        Self {
            client: client.clone(),
            bucket: bucket.to_string(),
            key: key.to_string(),
            tags,
        }
    }

    /// Execute and return the raw JSON response.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on S3 API failure.
    pub async fn run(&self) -> Result<Value, OperationError> {
        let sdk_tags = self
            .tags
            .iter()
            .map(|(k, v)| {
                SdkTag::builder()
                    .key(k)
                    .value(v)
                    .build()
                    .map_err(|e| OperationError::Http {
                        status: None,
                        message: format!("failed to build tag: {e}"),
                    })
            })
            .collect::<Result<Vec<SdkTag>, OperationError>>()?;

        let tagging = Tagging::builder()
            .set_tag_set(Some(sdk_tags))
            .build()
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("failed to build tagging: {e}"),
            })?;

        let resp = self
            .client
            .client()
            .put_object_tagging()
            .bucket(&self.bucket)
            .key(&self.key)
            .tagging(tagging)
            .send()
            .await
            .map_err(sdk_err)?;

        Ok(serde_json::json!({
            "version_id": resp.version_id(),
        }))
    }
}

#[async_trait]
impl Operation for PutObjectTagging {
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
            "tag_count": self.tags.len(),
        }))
    }
}

/// Remove all tags from an S3 object.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_s3::{S3Client, tagging::DeleteObjectTagging};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let s3 = S3Client::from_credentials("AKID", "SECRET", "eu-west-1", None)?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = DeleteObjectTagging::new(&s3, "my-bucket", "path/to/file.txt");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct DeleteObjectTagging {
    client: S3Client,
    bucket: String,
    key: String,
}

impl DeleteObjectTagging {
    /// Create a new delete-object-tagging operation.
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
            .delete_object_tagging()
            .bucket(&self.bucket)
            .key(&self.key)
            .send()
            .await
            .map_err(sdk_err)?;

        Ok(serde_json::json!({
            "version_id": resp.version_id(),
        }))
    }
}

#[async_trait]
impl Operation for DeleteObjectTagging {
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
