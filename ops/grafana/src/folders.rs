//! Folder operations.
//!
//! Provides CRUD and permissions for Grafana folders
//! via the `/api/folders/` endpoints.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::client::GrafanaClient;
use crate::helpers::{delete, get, post, put, to_value};

/// Folder metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderOutput {
    /// Folder numeric ID.
    pub id: Option<u64>,
    /// Folder UID.
    pub uid: Option<String>,
    /// Folder title.
    pub title: Option<String>,
    /// Folder URL.
    pub url: Option<String>,
}

/// A folder permission entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderPermission {
    /// Folder numeric ID.
    pub folder_id: Option<u64>,
    /// Role.
    pub role: Option<String>,
    /// Permission level.
    pub permission: Option<u64>,
    /// Team ID.
    pub team_id: Option<u64>,
    /// User ID.
    pub user_id: Option<u64>,
}

/// Create a folder.
///
/// Sends a `POST /api/folders` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::folders::FolderCreate;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = FolderCreate::new(&grafana, "My Folder", Some("my-folder-uid"));
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct FolderCreate {
    url: String,
    token: String,
    http: reqwest::Client,
    title: String,
    uid: Option<String>,
}

impl FolderCreate {
    /// Create a new folder operation.
    pub fn new(client: &GrafanaClient, title: &str, uid: Option<&str>) -> Self {
        Self {
            url: client.url("/api/folders"),
            token: client.token().to_string(),
            http: client.http().clone(),
            title: title.to_string(),
            uid: uid.map(String::from),
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on API failure.
    pub async fn run(&self) -> Result<FolderOutput, OperationError> {
        let mut body = serde_json::json!({ "title": self.title });
        if let Some(uid) = &self.uid {
            body["uid"] = Value::String(uid.clone());
        }
        post(&self.http, &self.url, &self.token, &body).await
    }
}

#[async_trait]
impl Operation for FolderCreate {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run().await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "title": self.title, "uid": self.uid }))
    }
}

impl TypedOperation for FolderCreate {
    type Output = FolderOutput;
}

/// Get a folder by UID.
///
/// Sends a `GET /api/folders/{uid}` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::folders::FolderGet;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = FolderGet::new(&grafana, "my-folder-uid");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct FolderGet {
    url: String,
    token: String,
    http: reqwest::Client,
    uid: String,
}

impl FolderGet {
    /// Create a get-folder operation.
    pub fn new(client: &GrafanaClient, uid: &str) -> Self {
        Self {
            url: client.url(&format!("/api/folders/{uid}")),
            token: client.token().to_string(),
            http: client.http().clone(),
            uid: uid.to_string(),
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on API failure.
    pub async fn run(&self) -> Result<FolderOutput, OperationError> {
        get(&self.http, &self.url, &self.token).await
    }
}

#[async_trait]
impl Operation for FolderGet {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run().await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "uid": self.uid }))
    }
}

impl TypedOperation for FolderGet {
    type Output = FolderOutput;
}

/// Update a folder.
///
/// Sends a `PUT /api/folders/{uid}` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::folders::FolderUpdate;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = FolderUpdate::new(&grafana, "my-uid", "New Title", 1);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct FolderUpdate {
    url: String,
    token: String,
    http: reqwest::Client,
    uid: String,
    title: String,
    version: u64,
}

impl FolderUpdate {
    /// Create an update-folder operation.
    pub fn new(client: &GrafanaClient, uid: &str, title: &str, version: u64) -> Self {
        Self {
            url: client.url(&format!("/api/folders/{uid}")),
            token: client.token().to_string(),
            http: client.http().clone(),
            uid: uid.to_string(),
            title: title.to_string(),
            version,
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on API failure.
    pub async fn run(&self) -> Result<FolderOutput, OperationError> {
        let body = serde_json::json!({ "title": self.title, "version": self.version });
        put(&self.http, &self.url, &self.token, &body).await
    }
}

#[async_trait]
impl Operation for FolderUpdate {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run().await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "uid": self.uid, "title": self.title }))
    }
}

impl TypedOperation for FolderUpdate {
    type Output = FolderOutput;
}

/// Delete a folder by UID.
///
/// Sends a `DELETE /api/folders/{uid}` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::folders::FolderDelete;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = FolderDelete::new(&grafana, "my-uid");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct FolderDelete {
    url: String,
    token: String,
    http: reqwest::Client,
    uid: String,
}

impl FolderDelete {
    /// Create a delete-folder operation.
    pub fn new(client: &GrafanaClient, uid: &str) -> Self {
        Self {
            url: client.url(&format!("/api/folders/{uid}")),
            token: client.token().to_string(),
            http: client.http().clone(),
            uid: uid.to_string(),
        }
    }

    /// Execute and return the raw JSON.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on API failure.
    pub async fn run(&self) -> Result<Value, OperationError> {
        delete(&self.http, &self.url, &self.token).await
    }
}

#[async_trait]
impl Operation for FolderDelete {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        self.run().await
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "uid": self.uid }))
    }
}

/// Get permissions for a folder.
///
/// Sends a `GET /api/folders/{uid}/permissions` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::folders::FolderGetPermissions;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = FolderGetPermissions::new(&grafana, "my-uid");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct FolderGetPermissions {
    url: String,
    token: String,
    http: reqwest::Client,
    uid: String,
}

impl FolderGetPermissions {
    /// Create a get-permissions operation.
    pub fn new(client: &GrafanaClient, uid: &str) -> Self {
        Self {
            url: client.url(&format!("/api/folders/{uid}/permissions")),
            token: client.token().to_string(),
            http: client.http().clone(),
            uid: uid.to_string(),
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on API failure.
    pub async fn run(&self) -> Result<Vec<FolderPermission>, OperationError> {
        get(&self.http, &self.url, &self.token).await
    }
}

#[async_trait]
impl Operation for FolderGetPermissions {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run().await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "uid": self.uid }))
    }
}

impl TypedOperation for FolderGetPermissions {
    type Output = Vec<FolderPermission>;
}

/// Update permissions for a folder.
///
/// Sends a `POST /api/folders/{uid}/permissions` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::folders::FolderUpdatePermissions;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use serde_json::json;
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let items = json!({"items": [{"role": "Viewer", "permission": 1}]});
/// let op = FolderUpdatePermissions::new(&grafana, "my-uid", items);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct FolderUpdatePermissions {
    url: String,
    token: String,
    http: reqwest::Client,
    uid: String,
    body: Value,
}

impl FolderUpdatePermissions {
    /// Create an update-permissions operation.
    pub fn new(client: &GrafanaClient, uid: &str, body: Value) -> Self {
        Self {
            url: client.url(&format!("/api/folders/{uid}/permissions")),
            token: client.token().to_string(),
            http: client.http().clone(),
            uid: uid.to_string(),
            body,
        }
    }

    /// Execute and return the raw JSON.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on API failure.
    pub async fn run(&self) -> Result<Value, OperationError> {
        post::<_, Value>(&self.http, &self.url, &self.token, &self.body).await
    }
}

#[async_trait]
impl Operation for FolderUpdatePermissions {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        self.run().await
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "uid": self.uid, "body": self.body }))
    }
}
