//! Configuration and build info operations.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext};
use serde_json::{Value, json};

use crate::MimirClient;
use crate::error::check_response;

/// Retrieve Mimir's runtime configuration.
///
/// Calls `GET /config`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_mimir::{MimirClient, status::GetConfig};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let mimir = MimirClient::from_context(&ctx).await?;
/// let op = GetConfig::new(mimir);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetConfig {
    client: MimirClient,
}

impl GetConfig {
    /// Create a new config query.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_mimir::{MimirClient, status::GetConfig};
    /// use reqwest::Client;
    ///
    /// let mimir = MimirClient::new("http://mimir:8080", Client::new());
    /// let op = GetConfig::new(mimir);
    /// ```
    pub fn new(client: MimirClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Operation for GetConfig {
    fn kind(&self) -> &str {
        "mimir"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({"operation": "get_config"}))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response =
            self.client
                .get("/config")
                .send()
                .await
                .map_err(|e| OperationError::Http {
                    status: None,
                    message: format!("get config request failed: {e}"),
                })?;

        let body = check_response(response).await?;
        let text = String::from_utf8_lossy(&body);
        Ok(json!({"config": text.as_ref()}))
    }
}

/// Retrieve only the non-default configuration values.
///
/// Calls `GET /config?mode=diff`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_mimir::{MimirClient, status::GetConfigDiff};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let mimir = MimirClient::from_context(&ctx).await?;
/// let op = GetConfigDiff::new(mimir);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetConfigDiff {
    client: MimirClient,
}

impl GetConfigDiff {
    /// Create a new config diff query.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_mimir::{MimirClient, status::GetConfigDiff};
    /// use reqwest::Client;
    ///
    /// let mimir = MimirClient::new("http://mimir:8080", Client::new());
    /// let op = GetConfigDiff::new(mimir);
    /// ```
    pub fn new(client: MimirClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Operation for GetConfigDiff {
    fn kind(&self) -> &str {
        "mimir"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({"operation": "get_config_diff"}))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response = self
            .client
            .get("/config")
            .query(&[("mode", "diff")])
            .send()
            .await
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("get config diff request failed: {e}"),
            })?;

        let body = check_response(response).await?;
        let text = String::from_utf8_lossy(&body);
        Ok(json!({"config_diff": text.as_ref()}))
    }
}

/// Retrieve the build information.
///
/// Calls `GET /api/v1/status/buildinfo`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_mimir::{MimirClient, status::GetBuildInfo};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let mimir = MimirClient::from_context(&ctx).await?;
/// let op = GetBuildInfo::new(mimir);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetBuildInfo {
    client: MimirClient,
}

impl GetBuildInfo {
    /// Create a new build info query.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_mimir::{MimirClient, status::GetBuildInfo};
    /// use reqwest::Client;
    ///
    /// let mimir = MimirClient::new("http://mimir:8080", Client::new());
    /// let op = GetBuildInfo::new(mimir);
    /// ```
    pub fn new(client: MimirClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Operation for GetBuildInfo {
    fn kind(&self) -> &str {
        "mimir"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({"operation": "get_build_info"}))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response = self
            .client
            .get("/api/v1/status/buildinfo")
            .send()
            .await
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("get build info request failed: {e}"),
            })?;

        let body = check_response(response).await?;
        serde_json::from_slice(&body).map_err(|e| OperationError::Deserialize {
            target_type: "Value".into(),
            reason: e.to_string(),
        })
    }
}

/// Retrieve the per-user runtime limits.
///
/// Calls `GET /runtime_config` to fetch limits applied to the current tenant.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_mimir::{MimirClient, status::GetUserLimits};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let mimir = MimirClient::from_context(&ctx).await?;
/// let op = GetUserLimits::new(mimir);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetUserLimits {
    client: MimirClient,
}

impl GetUserLimits {
    /// Create a new user limits query.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_mimir::{MimirClient, status::GetUserLimits};
    /// use reqwest::Client;
    ///
    /// let mimir = MimirClient::new("http://mimir:8080", Client::new());
    /// let op = GetUserLimits::new(mimir);
    /// ```
    pub fn new(client: MimirClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Operation for GetUserLimits {
    fn kind(&self) -> &str {
        "mimir"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({"operation": "get_user_limits"}))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response = self
            .client
            .get("/runtime_config")
            .send()
            .await
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("get user limits request failed: {e}"),
            })?;

        let body = check_response(response).await?;
        let text = String::from_utf8_lossy(&body);
        Ok(json!({"limits": text.as_ref()}))
    }
}
