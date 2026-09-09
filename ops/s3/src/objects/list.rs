//! [`ListObjects`] operation (ListObjectsV2).

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::S3Client;
use crate::error::sdk_err;

/// A single S3 object entry from a list operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectEntry {
    /// Object key.
    pub key: Option<String>,
    /// Size in bytes.
    pub size: Option<i64>,
    /// ETag.
    pub etag: Option<String>,
    /// Last modification timestamp (RFC 3339).
    pub last_modified: Option<String>,
    /// Storage class.
    pub storage_class: Option<String>,
}

/// Output of a [`ListObjects`] operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListObjectsOutput {
    /// Object entries.
    pub contents: Vec<ObjectEntry>,
    /// Common prefixes (for delimiter-based listing).
    pub common_prefixes: Vec<String>,
    /// Continuation token for pagination.
    pub next_continuation_token: Option<String>,
    /// Whether the list was truncated.
    pub is_truncated: bool,
}

/// List objects in a bucket with optional prefix and pagination.
///
/// Uses the ListObjectsV2 API.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_s3::{S3Client, objects::ListObjects};
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let s3 = S3Client::from_credentials("AKID", "SECRET", "eu-west-1", None)?;
/// let output = ListObjects::new(&s3, "my-bucket")
///     .with_prefix("logs/")
///     .with_max_keys(100)
///     .run()
///     .await?;
/// # Ok(())
/// # }
/// ```
pub struct ListObjects {
    client: S3Client,
    bucket: String,
    prefix: Option<String>,
    delimiter: Option<String>,
    max_keys: Option<i32>,
    continuation_token: Option<String>,
}

impl ListObjects {
    /// Create a new list-objects operation.
    pub fn new(client: &S3Client, bucket: &str) -> Self {
        Self {
            client: client.clone(),
            bucket: bucket.to_string(),
            prefix: None,
            delimiter: None,
            max_keys: None,
            continuation_token: None,
        }
    }

    /// Filter results to keys starting with this prefix.
    pub fn with_prefix(mut self, prefix: &str) -> Self {
        self.prefix = Some(prefix.to_string());
        self
    }

    /// Group keys by this delimiter (commonly `/`).
    pub fn with_delimiter(mut self, delimiter: &str) -> Self {
        self.delimiter = Some(delimiter.to_string());
        self
    }

    /// Limit the number of results returned.
    pub fn with_max_keys(mut self, max_keys: i32) -> Self {
        self.max_keys = Some(max_keys);
        self
    }

    /// Continue listing from a previous pagination token.
    pub fn with_continuation_token(mut self, token: &str) -> Self {
        self.continuation_token = Some(token.to_string());
        self
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on S3 API failure.
    pub async fn run(&self) -> Result<ListObjectsOutput, OperationError> {
        let mut req = self.client.client().list_objects_v2().bucket(&self.bucket);

        if let Some(prefix) = &self.prefix {
            req = req.prefix(prefix);
        }
        if let Some(delimiter) = &self.delimiter {
            req = req.delimiter(delimiter);
        }
        if let Some(max_keys) = self.max_keys {
            req = req.max_keys(max_keys);
        }
        if let Some(token) = &self.continuation_token {
            req = req.continuation_token(token);
        }

        let resp = req.send().await.map_err(sdk_err)?;

        let contents = resp
            .contents()
            .iter()
            .map(|obj| ObjectEntry {
                key: obj.key().map(String::from),
                size: obj.size(),
                etag: obj.e_tag().map(String::from),
                last_modified: obj.last_modified().map(|t| t.to_string()),
                storage_class: obj.storage_class().map(|s| s.as_str().to_string()),
            })
            .collect();

        let common_prefixes = resp
            .common_prefixes()
            .iter()
            .filter_map(|p| p.prefix().map(String::from))
            .collect();

        Ok(ListObjectsOutput {
            contents,
            common_prefixes,
            next_continuation_token: resp.next_continuation_token().map(String::from),
            is_truncated: resp.is_truncated().unwrap_or(false),
        })
    }
}

#[async_trait]
impl Operation for ListObjects {
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
            "prefix": self.prefix,
            "delimiter": self.delimiter,
            "max_keys": self.max_keys,
        }))
    }
}

impl TypedOperation for ListObjects {
    type Output = ListObjectsOutput;
}
