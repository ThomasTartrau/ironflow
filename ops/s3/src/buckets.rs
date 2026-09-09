//! Bucket operations: create, delete, list, head, and get location.

use async_trait::async_trait;
use aws_sdk_s3::types::{BucketLocationConstraint, CreateBucketConfiguration};
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::S3Client;
use crate::error::sdk_err;

/// Output of a [`CreateBucket`] operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateBucketOutput {
    /// Location of the created bucket (e.g. `/my-bucket`).
    pub location: Option<String>,
}

/// Create an S3 bucket.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_s3::{S3Client, buckets::CreateBucket};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let s3 = S3Client::from_credentials("AKID", "SECRET", "eu-west-1", None)?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = CreateBucket::new(&s3, "my-new-bucket", Some("eu-west-1"));
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct CreateBucket {
    client: S3Client,
    bucket: String,
    region: Option<String>,
}

impl CreateBucket {
    /// Create a new create-bucket operation.
    pub fn new(client: &S3Client, bucket: &str, region: Option<&str>) -> Self {
        Self {
            client: client.clone(),
            bucket: bucket.to_string(),
            region: region.map(String::from),
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on S3 API failure.
    pub async fn run(&self) -> Result<CreateBucketOutput, OperationError> {
        let mut req = self.client.client().create_bucket().bucket(&self.bucket);

        if let Some(region) = &self.region
            && region != "us-east-1"
        {
            let constraint = BucketLocationConstraint::from(region.as_str());
            let config = CreateBucketConfiguration::builder()
                .location_constraint(constraint)
                .build();
            req = req.create_bucket_configuration(config);
        }

        let resp = req.send().await.map_err(sdk_err)?;

        Ok(CreateBucketOutput {
            location: resp.location().map(String::from),
        })
    }
}

#[async_trait]
impl Operation for CreateBucket {
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
            "region": self.region,
        }))
    }
}

impl TypedOperation for CreateBucket {
    type Output = CreateBucketOutput;
}

/// Delete an S3 bucket.
///
/// The bucket must be empty before deletion.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_s3::{S3Client, buckets::DeleteBucket};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let s3 = S3Client::from_credentials("AKID", "SECRET", "eu-west-1", None)?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = DeleteBucket::new(&s3, "my-bucket");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct DeleteBucket {
    client: S3Client,
    bucket: String,
}

impl DeleteBucket {
    /// Create a new delete-bucket operation.
    pub fn new(client: &S3Client, bucket: &str) -> Self {
        Self {
            client: client.clone(),
            bucket: bucket.to_string(),
        }
    }

    /// Execute the deletion.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on S3 API failure.
    pub async fn run(&self) -> Result<Value, OperationError> {
        self.client
            .client()
            .delete_bucket()
            .bucket(&self.bucket)
            .send()
            .await
            .map_err(sdk_err)?;

        Ok(serde_json::json!({ "deleted": self.bucket }))
    }
}

#[async_trait]
impl Operation for DeleteBucket {
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

/// A single bucket entry from a list operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BucketInfo {
    /// Bucket name.
    pub name: Option<String>,
    /// Creation timestamp (RFC 3339).
    pub creation_date: Option<String>,
}

/// Output of a [`ListBuckets`] operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListBucketsOutput {
    /// Bucket entries.
    pub buckets: Vec<BucketInfo>,
}

/// List all buckets owned by the authenticated user.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_s3::{S3Client, buckets::ListBuckets};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let s3 = S3Client::from_credentials("AKID", "SECRET", "eu-west-1", None)?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = ListBuckets::new(&s3);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct ListBuckets {
    client: S3Client,
}

impl ListBuckets {
    /// Create a new list-buckets operation.
    pub fn new(client: &S3Client) -> Self {
        Self {
            client: client.clone(),
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on S3 API failure.
    pub async fn run(&self) -> Result<ListBucketsOutput, OperationError> {
        let resp = self
            .client
            .client()
            .list_buckets()
            .send()
            .await
            .map_err(sdk_err)?;

        let buckets = resp
            .buckets()
            .iter()
            .map(|b| BucketInfo {
                name: b.name().map(String::from),
                creation_date: b.creation_date().map(|d| d.to_string()),
            })
            .collect();

        Ok(ListBucketsOutput { buckets })
    }
}

#[async_trait]
impl Operation for ListBuckets {
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
}

impl TypedOperation for ListBuckets {
    type Output = ListBucketsOutput;
}

/// Check whether a bucket exists and is accessible.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_s3::{S3Client, buckets::HeadBucket};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let s3 = S3Client::from_credentials("AKID", "SECRET", "eu-west-1", None)?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = HeadBucket::new(&s3, "my-bucket");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct HeadBucket {
    client: S3Client,
    bucket: String,
}

impl HeadBucket {
    /// Create a new head-bucket operation.
    pub fn new(client: &S3Client, bucket: &str) -> Self {
        Self {
            client: client.clone(),
            bucket: bucket.to_string(),
        }
    }

    /// Execute and confirm the bucket exists.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on S3 API failure (including 404 for
    /// non-existent buckets and 403 for access denied).
    pub async fn run(&self) -> Result<Value, OperationError> {
        self.client
            .client()
            .head_bucket()
            .bucket(&self.bucket)
            .send()
            .await
            .map_err(sdk_err)?;

        Ok(serde_json::json!({ "bucket": self.bucket, "exists": true }))
    }
}

#[async_trait]
impl Operation for HeadBucket {
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

/// Output of a [`GetBucketLocation`] operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetBucketLocationOutput {
    /// AWS region where the bucket resides.
    pub location: Option<String>,
}

/// Get the region where a bucket is located.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_s3::{S3Client, buckets::GetBucketLocation};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let s3 = S3Client::from_credentials("AKID", "SECRET", "eu-west-1", None)?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = GetBucketLocation::new(&s3, "my-bucket");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct GetBucketLocation {
    client: S3Client,
    bucket: String,
}

impl GetBucketLocation {
    /// Create a new get-bucket-location operation.
    pub fn new(client: &S3Client, bucket: &str) -> Self {
        Self {
            client: client.clone(),
            bucket: bucket.to_string(),
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on S3 API failure.
    pub async fn run(&self) -> Result<GetBucketLocationOutput, OperationError> {
        let resp = self
            .client
            .client()
            .get_bucket_location()
            .bucket(&self.bucket)
            .send()
            .await
            .map_err(sdk_err)?;

        Ok(GetBucketLocationOutput {
            location: resp.location_constraint().map(|l| l.as_str().to_string()),
        })
    }
}

#[async_trait]
impl Operation for GetBucketLocation {
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
        Some(serde_json::json!({ "bucket": self.bucket }))
    }
}

impl TypedOperation for GetBucketLocation {
    type Output = GetBucketLocationOutput;
}
