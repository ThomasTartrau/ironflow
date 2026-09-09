//! Alertmanager configuration write operations.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext};
use serde_json::{Value, json};

use crate::MimirClient;
use crate::error::check_response;

/// Set the Alertmanager configuration.
///
/// Calls `POST /api/v1/alerts` with a YAML configuration body.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_mimir::{MimirClient, alerts::SetAlertmanagerConfig};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let mimir = MimirClient::from_context(&ctx).await?;
/// let op = SetAlertmanagerConfig::new(mimir, "route:\n  receiver: default");
/// op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct SetAlertmanagerConfig {
    client: MimirClient,
    config: String,
}

impl SetAlertmanagerConfig {
    /// Create a new alertmanager config update.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_mimir::{MimirClient, alerts::SetAlertmanagerConfig};
    /// use reqwest::Client;
    ///
    /// let mimir = MimirClient::new("http://mimir:8080", Client::new());
    /// let op = SetAlertmanagerConfig::new(mimir, "route:\n  receiver: default");
    /// ```
    pub fn new(client: MimirClient, config: &str) -> Self {
        Self {
            client,
            config: config.to_owned(),
        }
    }
}

#[async_trait]
impl Operation for SetAlertmanagerConfig {
    fn kind(&self) -> &str {
        "mimir"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({"operation": "set_alertmanager_config"}))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails or the response
    /// status is not 2xx.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response = self
            .client
            .post("/api/v1/alerts")
            .header("Content-Type", "application/yaml")
            .body(self.config.clone())
            .send()
            .await
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("set alertmanager config request failed: {e}"),
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

/// Delete the Alertmanager configuration.
///
/// Calls `DELETE /api/v1/alerts`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_mimir::{MimirClient, alerts::DeleteAlertmanagerConfig};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let mimir = MimirClient::from_context(&ctx).await?;
/// let op = DeleteAlertmanagerConfig::new(mimir);
/// op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct DeleteAlertmanagerConfig {
    client: MimirClient,
}

impl DeleteAlertmanagerConfig {
    /// Create a new alertmanager config deletion.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_mimir::{MimirClient, alerts::DeleteAlertmanagerConfig};
    /// use reqwest::Client;
    ///
    /// let mimir = MimirClient::new("http://mimir:8080", Client::new());
    /// let op = DeleteAlertmanagerConfig::new(mimir);
    /// ```
    pub fn new(client: MimirClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Operation for DeleteAlertmanagerConfig {
    fn kind(&self) -> &str {
        "mimir"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({"operation": "delete_alertmanager_config"}))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails or the response
    /// status is not 2xx.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response = self
            .client
            .delete("/api/v1/alerts")
            .send()
            .await
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("delete alertmanager config request failed: {e}"),
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
