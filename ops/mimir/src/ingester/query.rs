//! Ingester read-only query operations.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext};
use serde_json::{Value, json};

use crate::MimirClient;
use crate::error::check_response;

/// Retrieve the ingester hash ring.
///
/// Calls `GET /ingester/ring`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_mimir::{MimirClient, ingester::GetIngesterRing};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let mimir = MimirClient::from_context(&ctx).await?;
/// let op = GetIngesterRing::new(mimir);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetIngesterRing {
    client: MimirClient,
}

impl GetIngesterRing {
    /// Create a new ingester ring query.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_mimir::{MimirClient, ingester::GetIngesterRing};
    /// use reqwest::Client;
    ///
    /// let mimir = MimirClient::new("http://mimir:8080", Client::new());
    /// let op = GetIngesterRing::new(mimir);
    /// ```
    pub fn new(client: MimirClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Operation for GetIngesterRing {
    fn kind(&self) -> &str {
        "mimir"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({"operation": "get_ingester_ring"}))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response = self
            .client
            .get("/ingester/ring")
            .send()
            .await
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("get ingester ring request failed: {e}"),
            })?;

        let body = check_response(response).await?;
        let text = String::from_utf8_lossy(&body);
        Ok(json!({"ring": text.as_ref()}))
    }
}

/// List tenants with data in the ingester.
///
/// Calls `GET /ingester/tenants`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_mimir::{MimirClient, ingester::GetIngesterTenants};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let mimir = MimirClient::from_context(&ctx).await?;
/// let op = GetIngesterTenants::new(mimir);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetIngesterTenants {
    client: MimirClient,
}

impl GetIngesterTenants {
    /// Create a new ingester tenants query.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_mimir::{MimirClient, ingester::GetIngesterTenants};
    /// use reqwest::Client;
    ///
    /// let mimir = MimirClient::new("http://mimir:8080", Client::new());
    /// let op = GetIngesterTenants::new(mimir);
    /// ```
    pub fn new(client: MimirClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Operation for GetIngesterTenants {
    fn kind(&self) -> &str {
        "mimir"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({"operation": "get_ingester_tenants"}))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response = self
            .client
            .get("/ingester/tenants")
            .send()
            .await
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("get ingester tenants request failed: {e}"),
            })?;

        let body = check_response(response).await?;
        serde_json::from_slice(&body).map_err(|e| OperationError::Deserialize {
            target_type: "Value".into(),
            reason: e.to_string(),
        })
    }
}
