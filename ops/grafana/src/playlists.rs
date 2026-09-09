//! Playlist operations.
//!
//! Provides CRUD for Grafana playlists via the `/api/playlists/` endpoints.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::client::GrafanaClient;
use crate::helpers::{delete, get, post, put, to_value};

/// Playlist metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistOutput {
    /// Playlist numeric ID.
    pub id: Option<u64>,
    /// Playlist UID.
    pub uid: Option<String>,
    /// Playlist name.
    pub name: Option<String>,
    /// Playback interval (e.g. "5m").
    pub interval: Option<String>,
}

/// List all playlists.
///
/// Sends a `GET /api/playlists` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::playlists::PlaylistList;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = PlaylistList::new(&grafana);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct PlaylistList {
    url: String,
    token: String,
    http: reqwest::Client,
}

impl PlaylistList {
    /// Create a list operation.
    pub fn new(client: &GrafanaClient) -> Self {
        Self {
            url: client.url("/api/playlists"),
            token: client.token().to_string(),
            http: client.http().clone(),
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on API failure.
    pub async fn run(&self) -> Result<Vec<PlaylistOutput>, OperationError> {
        get(&self.http, &self.url, &self.token).await
    }
}

#[async_trait]
impl Operation for PlaylistList {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run().await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "endpoint": "playlists" }))
    }
}

impl TypedOperation for PlaylistList {
    type Output = Vec<PlaylistOutput>;
}

/// Get a playlist by UID.
///
/// Sends a `GET /api/playlists/{uid}` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::playlists::PlaylistGet;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = PlaylistGet::new(&grafana, "my-playlist-uid");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct PlaylistGet {
    url: String,
    token: String,
    http: reqwest::Client,
    uid: String,
}

impl PlaylistGet {
    /// Create a get operation.
    pub fn new(client: &GrafanaClient, uid: &str) -> Self {
        Self {
            url: client.url(&format!("/api/playlists/{uid}")),
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
    pub async fn run(&self) -> Result<PlaylistOutput, OperationError> {
        get(&self.http, &self.url, &self.token).await
    }
}

#[async_trait]
impl Operation for PlaylistGet {
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

impl TypedOperation for PlaylistGet {
    type Output = PlaylistOutput;
}

/// Create a playlist.
///
/// Sends a `POST /api/playlists` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::playlists::PlaylistCreate;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use serde_json::json;
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let body = json!({"name": "My Playlist", "interval": "5m", "items": []});
/// let op = PlaylistCreate::new(&grafana, body);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct PlaylistCreate {
    url: String,
    token: String,
    http: reqwest::Client,
    body: Value,
}

impl PlaylistCreate {
    /// Create a playlist creation operation.
    pub fn new(client: &GrafanaClient, body: Value) -> Self {
        Self {
            url: client.url("/api/playlists"),
            token: client.token().to_string(),
            http: client.http().clone(),
            body,
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on API failure.
    pub async fn run(&self) -> Result<PlaylistOutput, OperationError> {
        post(&self.http, &self.url, &self.token, &self.body).await
    }
}

#[async_trait]
impl Operation for PlaylistCreate {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run().await?)
    }

    fn input(&self) -> Option<Value> {
        Some(self.body.clone())
    }
}

impl TypedOperation for PlaylistCreate {
    type Output = PlaylistOutput;
}

/// Update a playlist.
///
/// Sends a `PUT /api/playlists/{uid}` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::playlists::PlaylistUpdate;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use serde_json::json;
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let body = json!({"name": "Updated", "interval": "10m", "items": []});
/// let op = PlaylistUpdate::new(&grafana, "my-uid", body);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct PlaylistUpdate {
    url: String,
    token: String,
    http: reqwest::Client,
    uid: String,
    body: Value,
}

impl PlaylistUpdate {
    /// Create an update operation.
    pub fn new(client: &GrafanaClient, uid: &str, body: Value) -> Self {
        Self {
            url: client.url(&format!("/api/playlists/{uid}")),
            token: client.token().to_string(),
            http: client.http().clone(),
            uid: uid.to_string(),
            body,
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] on API failure.
    pub async fn run(&self) -> Result<PlaylistOutput, OperationError> {
        put(&self.http, &self.url, &self.token, &self.body).await
    }
}

#[async_trait]
impl Operation for PlaylistUpdate {
    fn kind(&self) -> &str {
        "grafana"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run().await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "uid": self.uid, "body": self.body }))
    }
}

impl TypedOperation for PlaylistUpdate {
    type Output = PlaylistOutput;
}

/// Delete a playlist.
///
/// Sends a `DELETE /api/playlists/{uid}` request.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_grafana::GrafanaClient;
/// use ironflow_ops_grafana::playlists::PlaylistDelete;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let grafana = GrafanaClient::new("token", "http://localhost:3000")?;
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let op = PlaylistDelete::new(&grafana, "my-uid");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct PlaylistDelete {
    url: String,
    token: String,
    http: reqwest::Client,
    uid: String,
}

impl PlaylistDelete {
    /// Create a delete operation.
    pub fn new(client: &GrafanaClient, uid: &str) -> Self {
        Self {
            url: client.url(&format!("/api/playlists/{uid}")),
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
impl Operation for PlaylistDelete {
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
