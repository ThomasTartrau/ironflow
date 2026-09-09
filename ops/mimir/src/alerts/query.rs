//! Read-only alertmanager operations.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext};
use serde_json::{Value, json};

use crate::MimirClient;
use crate::error::check_response;

/// Retrieve active alerts.
///
/// Calls `GET /prometheus/api/v1/alerts`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_mimir::{MimirClient, alerts::GetAlerts};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let mimir = MimirClient::from_context(&ctx).await?;
/// let op = GetAlerts::new(mimir);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetAlerts {
    client: MimirClient,
}

impl GetAlerts {
    /// Create a new alerts query.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_mimir::{MimirClient, alerts::GetAlerts};
    /// use reqwest::Client;
    ///
    /// let mimir = MimirClient::new("http://mimir:8080", Client::new());
    /// let op = GetAlerts::new(mimir);
    /// ```
    pub fn new(client: MimirClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Operation for GetAlerts {
    fn kind(&self) -> &str {
        "mimir"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({"operation": "get_alerts"}))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails or the response
    /// status is not 2xx.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response = self
            .client
            .get("/prometheus/api/v1/alerts")
            .send()
            .await
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("get alerts request failed: {e}"),
            })?;

        let body = check_response(response).await?;
        serde_json::from_slice(&body).map_err(|e| OperationError::Deserialize {
            target_type: "Value".into(),
            reason: e.to_string(),
        })
    }
}

/// Retrieve the current Alertmanager configuration.
///
/// Calls `GET /api/v1/alerts`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_mimir::{MimirClient, alerts::GetAlertmanagerConfig};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let mimir = MimirClient::from_context(&ctx).await?;
/// let op = GetAlertmanagerConfig::new(mimir);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetAlertmanagerConfig {
    client: MimirClient,
}

impl GetAlertmanagerConfig {
    /// Create a new alertmanager config query.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_mimir::{MimirClient, alerts::GetAlertmanagerConfig};
    /// use reqwest::Client;
    ///
    /// let mimir = MimirClient::new("http://mimir:8080", Client::new());
    /// let op = GetAlertmanagerConfig::new(mimir);
    /// ```
    pub fn new(client: MimirClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Operation for GetAlertmanagerConfig {
    fn kind(&self) -> &str {
        "mimir"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({"operation": "get_alertmanager_config"}))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails or the response
    /// status is not 2xx.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response = self
            .client
            .get("/api/v1/alerts")
            .send()
            .await
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("get alertmanager config request failed: {e}"),
            })?;

        let body = check_response(response).await?;
        serde_json::from_slice(&body).map_err(|e| OperationError::Deserialize {
            target_type: "Value".into(),
            reason: e.to_string(),
        })
    }
}

/// Retrieve the Alertmanager status.
///
/// Calls `GET /multitenant_alertmanager/status`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_mimir::{MimirClient, alerts::GetAlertmanagerStatus};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let mimir = MimirClient::from_context(&ctx).await?;
/// let op = GetAlertmanagerStatus::new(mimir);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetAlertmanagerStatus {
    client: MimirClient,
}

impl GetAlertmanagerStatus {
    /// Create a new alertmanager status query.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_mimir::{MimirClient, alerts::GetAlertmanagerStatus};
    /// use reqwest::Client;
    ///
    /// let mimir = MimirClient::new("http://mimir:8080", Client::new());
    /// let op = GetAlertmanagerStatus::new(mimir);
    /// ```
    pub fn new(client: MimirClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Operation for GetAlertmanagerStatus {
    fn kind(&self) -> &str {
        "mimir"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({"operation": "get_alertmanager_status"}))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails or the response
    /// status is not 2xx.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response = self
            .client
            .get("/multitenant_alertmanager/status")
            .send()
            .await
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("get alertmanager status request failed: {e}"),
            })?;

        let body = check_response(response).await?;
        serde_json::from_slice(&body).map_err(|e| OperationError::Deserialize {
            target_type: "Value".into(),
            reason: e.to_string(),
        })
    }
}

/// List Alertmanager configurations for all tenants (admin endpoint).
///
/// Calls `GET /multitenant_alertmanager/configs`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_mimir::{MimirClient, alerts::GetAlertmanagerConfigs};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let mimir = MimirClient::from_context(&ctx).await?;
/// let op = GetAlertmanagerConfigs::new(mimir);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetAlertmanagerConfigs {
    client: MimirClient,
}

impl GetAlertmanagerConfigs {
    /// Create a new alertmanager configs listing.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_mimir::{MimirClient, alerts::GetAlertmanagerConfigs};
    /// use reqwest::Client;
    ///
    /// let mimir = MimirClient::new("http://mimir:8080", Client::new());
    /// let op = GetAlertmanagerConfigs::new(mimir);
    /// ```
    pub fn new(client: MimirClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Operation for GetAlertmanagerConfigs {
    fn kind(&self) -> &str {
        "mimir"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({"operation": "get_alertmanager_configs"}))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails or the response
    /// status is not 2xx.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response = self
            .client
            .get("/multitenant_alertmanager/configs")
            .send()
            .await
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("get alertmanager configs request failed: {e}"),
            })?;

        let body = check_response(response).await?;
        serde_json::from_slice(&body).map_err(|e| OperationError::Deserialize {
            target_type: "Value".into(),
            reason: e.to_string(),
        })
    }
}
