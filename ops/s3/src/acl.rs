//! ACL and restore operations for S3 objects.

use async_trait::async_trait;
use aws_sdk_s3::types::{ObjectCannedAcl, RestoreRequest};
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::S3Client;
use crate::error::sdk_err;

/// Owner information from an ACL response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OwnerInfo {
    /// Canonical user ID.
    pub id: Option<String>,
    /// Display name.
    pub display_name: Option<String>,
}

/// A single grant entry from an ACL response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrantInfo {
    /// Permission level (e.g. `FULL_CONTROL`, `READ`).
    pub permission: Option<String>,
    /// Grantee type (e.g. `CanonicalUser`, `Group`).
    pub grantee_type: Option<String>,
    /// Grantee identifier (canonical user ID or group URI).
    pub grantee_id: Option<String>,
}

/// Output of a [`GetObjectAcl`] operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetObjectAclOutput {
    /// Object owner.
    pub owner: Option<OwnerInfo>,
    /// Access control grants.
    pub grants: Vec<GrantInfo>,
}

/// Retrieve the access control list for an object.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_s3::{S3Client, acl::GetObjectAcl};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let s3 = S3Client::from_credentials("AKID", "SECRET", "eu-west-1", None)?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = GetObjectAcl::new(&s3, "my-bucket", "my-key");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct GetObjectAcl {
    client: S3Client,
    bucket: String,
    key: String,
}

impl GetObjectAcl {
    /// Create a new get-object-acl operation.
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
    pub async fn run(&self) -> Result<GetObjectAclOutput, OperationError> {
        let resp = self
            .client
            .client()
            .get_object_acl()
            .bucket(&self.bucket)
            .key(&self.key)
            .send()
            .await
            .map_err(sdk_err)?;

        let owner = resp.owner().map(|o| OwnerInfo {
            id: o.id().map(String::from),
            display_name: o.display_name().map(String::from),
        });

        let grants = resp
            .grants()
            .iter()
            .map(|g| {
                let (grantee_type, grantee_id) = match g.grantee() {
                    Some(grantee) => (
                        Some(grantee.r#type().as_str().to_string()),
                        grantee.id().map(String::from),
                    ),
                    None => (None, None),
                };
                GrantInfo {
                    permission: g.permission().map(|p| p.as_str().to_string()),
                    grantee_type,
                    grantee_id,
                }
            })
            .collect();

        Ok(GetObjectAclOutput { owner, grants })
    }
}

#[async_trait]
impl Operation for GetObjectAcl {
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

impl TypedOperation for GetObjectAcl {
    type Output = GetObjectAclOutput;
}

/// Set the canned ACL for an object.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_s3::{S3Client, acl::PutObjectAcl};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let s3 = S3Client::from_credentials("AKID", "SECRET", "eu-west-1", None)?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = PutObjectAcl::new(&s3, "my-bucket", "my-key", "private");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct PutObjectAcl {
    client: S3Client,
    bucket: String,
    key: String,
    acl: String,
}

impl PutObjectAcl {
    /// Create a new put-object-acl operation.
    pub fn new(client: &S3Client, bucket: &str, key: &str, acl: &str) -> Self {
        Self {
            client: client.clone(),
            bucket: bucket.to_string(),
            key: key.to_string(),
            acl: acl.to_string(),
        }
    }

    /// Execute and return the raw JSON response.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on S3 API failure.
    pub async fn run(&self) -> Result<Value, OperationError> {
        self.client
            .client()
            .put_object_acl()
            .bucket(&self.bucket)
            .key(&self.key)
            .acl(ObjectCannedAcl::from(self.acl.as_str()))
            .send()
            .await
            .map_err(sdk_err)?;

        Ok(serde_json::json!({}))
    }
}

#[async_trait]
impl Operation for PutObjectAcl {
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
            "acl": self.acl,
        }))
    }
}

/// Initiate a restore request for an archived object (e.g. from Glacier).
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_s3::{S3Client, acl::RestoreObject};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let s3 = S3Client::from_credentials("AKID", "SECRET", "eu-west-1", None)?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = RestoreObject::new(&s3, "my-bucket", "archived-key", 7);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct RestoreObject {
    client: S3Client,
    bucket: String,
    key: String,
    days: i32,
}

impl RestoreObject {
    /// Create a new restore-object operation.
    pub fn new(client: &S3Client, bucket: &str, key: &str, days: i32) -> Self {
        Self {
            client: client.clone(),
            bucket: bucket.to_string(),
            key: key.to_string(),
            days,
        }
    }

    /// Execute and return the raw JSON response.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on S3 API failure.
    pub async fn run(&self) -> Result<Value, OperationError> {
        let restore_request = RestoreRequest::builder().days(self.days).build();

        let resp = self
            .client
            .client()
            .restore_object()
            .bucket(&self.bucket)
            .key(&self.key)
            .restore_request(restore_request)
            .send()
            .await
            .map_err(sdk_err)?;

        Ok(serde_json::json!({
            "restore_output_path": resp.restore_output_path(),
        }))
    }
}

#[async_trait]
impl Operation for RestoreObject {
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
            "days": self.days,
        }))
    }
}
