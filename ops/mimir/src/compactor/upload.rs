//! Block upload operations for the compactor.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext};
use serde_json::{Value, json};

use crate::MimirClient;
use crate::error::{check_response, validate_path_segment};

/// Start a block upload to the compactor.
///
/// Calls `POST /api/v1/upload/block/{block_id}/start`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_mimir::{MimirClient, compactor::StartBlockUpload};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let mimir = MimirClient::from_context(&ctx).await?;
/// let op = StartBlockUpload::new(mimir, "01ABCDEF12345678");
/// op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct StartBlockUpload {
    client: MimirClient,
    block_id: String,
}

impl StartBlockUpload {
    /// Create a new start block upload operation.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_mimir::{MimirClient, compactor::StartBlockUpload};
    /// use reqwest::Client;
    ///
    /// let mimir = MimirClient::new("http://mimir:8080", Client::new());
    /// let op = StartBlockUpload::new(mimir, "01ABCDEF12345678");
    /// ```
    pub fn new(client: MimirClient, block_id: &str) -> Self {
        Self {
            client,
            block_id: block_id.to_owned(),
        }
    }
}

#[async_trait]
impl Operation for StartBlockUpload {
    fn kind(&self) -> &str {
        "mimir"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "operation": "start_block_upload",
            "block_id": self.block_id,
        }))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails or the block ID
    /// contains path-traversal characters.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        validate_path_segment(&self.block_id, "block_id", "mimir")?;
        let path = format!("/api/v1/upload/block/{}/start", self.block_id);
        let response = self
            .client
            .post(&path)
            .send()
            .await
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("start block upload request failed: {e}"),
            })?;

        let body = check_response(response).await?;
        if body.is_empty() {
            return Ok(json!({"status": "success"}));
        }
        serde_json::from_slice(&body).map_err(|e| OperationError::Deserialize {
            target_type: "Value".into(),
            reason: e.to_string(),
        })
    }
}

/// Upload a file to a block upload.
///
/// Calls `POST /api/v1/upload/block/{block_id}/files` with the file data.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_mimir::{MimirClient, compactor::UploadBlockFile};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let mimir = MimirClient::from_context(&ctx).await?;
/// let op = UploadBlockFile::new(mimir, "01ABCDEF12345678", "index", vec![1, 2, 3]);
/// op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct UploadBlockFile {
    client: MimirClient,
    block_id: String,
    path: String,
    data: Vec<u8>,
}

impl UploadBlockFile {
    /// Create a new block file upload operation.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_mimir::{MimirClient, compactor::UploadBlockFile};
    /// use reqwest::Client;
    ///
    /// let mimir = MimirClient::new("http://mimir:8080", Client::new());
    /// let op = UploadBlockFile::new(mimir, "01ABCDEF12345678", "index", vec![1, 2, 3]);
    /// ```
    pub fn new(client: MimirClient, block_id: &str, path: &str, data: Vec<u8>) -> Self {
        Self {
            client,
            block_id: block_id.to_owned(),
            path: path.to_owned(),
            data,
        }
    }
}

#[async_trait]
impl Operation for UploadBlockFile {
    fn kind(&self) -> &str {
        "mimir"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "operation": "upload_block_file",
            "block_id": self.block_id,
            "path": self.path,
            "data_size": self.data.len(),
        }))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails or the block ID
    /// contains path-traversal characters.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        validate_path_segment(&self.block_id, "block_id", "mimir")?;
        let url = format!("/api/v1/upload/block/{}/files", self.block_id);
        let response = self
            .client
            .post(&url)
            .query(&[("path", &self.path)])
            .body(self.data.clone())
            .send()
            .await
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("upload block file request failed: {e}"),
            })?;

        let body = check_response(response).await?;
        if body.is_empty() {
            return Ok(json!({"status": "success"}));
        }
        serde_json::from_slice(&body).map_err(|e| OperationError::Deserialize {
            target_type: "Value".into(),
            reason: e.to_string(),
        })
    }
}

/// Finish a block upload.
///
/// Calls `POST /api/v1/upload/block/{block_id}/finish`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_mimir::{MimirClient, compactor::FinishBlockUpload};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let mimir = MimirClient::from_context(&ctx).await?;
/// let op = FinishBlockUpload::new(mimir, "01ABCDEF12345678");
/// op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct FinishBlockUpload {
    client: MimirClient,
    block_id: String,
}

impl FinishBlockUpload {
    /// Create a new finish block upload operation.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_mimir::{MimirClient, compactor::FinishBlockUpload};
    /// use reqwest::Client;
    ///
    /// let mimir = MimirClient::new("http://mimir:8080", Client::new());
    /// let op = FinishBlockUpload::new(mimir, "01ABCDEF12345678");
    /// ```
    pub fn new(client: MimirClient, block_id: &str) -> Self {
        Self {
            client,
            block_id: block_id.to_owned(),
        }
    }
}

#[async_trait]
impl Operation for FinishBlockUpload {
    fn kind(&self) -> &str {
        "mimir"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "operation": "finish_block_upload",
            "block_id": self.block_id,
        }))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails or the block ID
    /// contains path-traversal characters.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        validate_path_segment(&self.block_id, "block_id", "mimir")?;
        let path = format!("/api/v1/upload/block/{}/finish", self.block_id);
        let response = self
            .client
            .post(&path)
            .send()
            .await
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("finish block upload request failed: {e}"),
            })?;

        let body = check_response(response).await?;
        if body.is_empty() {
            return Ok(json!({"status": "success"}));
        }
        serde_json::from_slice(&body).map_err(|e| OperationError::Deserialize {
            target_type: "Value".into(),
            reason: e.to_string(),
        })
    }
}
