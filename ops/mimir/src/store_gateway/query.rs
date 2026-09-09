//! Store-gateway read-only operations.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext};
use serde_json::{Value, json};

use crate::MimirClient;
use crate::error::{check_response, validate_path_segment};

/// Retrieve the store-gateway hash ring.
///
/// Calls `GET /store-gateway/ring`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_mimir::{MimirClient, store_gateway::GetRing};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let mimir = MimirClient::from_context(&ctx).await?;
/// let op = GetRing::new(mimir);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetRing {
    client: MimirClient,
}

impl GetRing {
    /// Create a new store-gateway ring query.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_mimir::{MimirClient, store_gateway::GetRing};
    /// use reqwest::Client;
    ///
    /// let mimir = MimirClient::new("http://mimir:8080", Client::new());
    /// let op = GetRing::new(mimir);
    /// ```
    pub fn new(client: MimirClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Operation for GetRing {
    fn kind(&self) -> &str {
        "mimir"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({"operation": "store_gateway_get_ring"}))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response = self
            .client
            .get("/store-gateway/ring")
            .send()
            .await
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("get store-gateway ring request failed: {e}"),
            })?;

        let body = check_response(response).await?;
        let text = String::from_utf8_lossy(&body);
        Ok(json!({"ring": text.as_ref()}))
    }
}

/// List tenants with blocks in the store-gateway.
///
/// Calls `GET /store-gateway/tenants`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_mimir::{MimirClient, store_gateway::GetTenants};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let mimir = MimirClient::from_context(&ctx).await?;
/// let op = GetTenants::new(mimir);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetTenants {
    client: MimirClient,
}

impl GetTenants {
    /// Create a new store-gateway tenants query.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_mimir::{MimirClient, store_gateway::GetTenants};
    /// use reqwest::Client;
    ///
    /// let mimir = MimirClient::new("http://mimir:8080", Client::new());
    /// let op = GetTenants::new(mimir);
    /// ```
    pub fn new(client: MimirClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Operation for GetTenants {
    fn kind(&self) -> &str {
        "mimir"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({"operation": "store_gateway_get_tenants"}))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response = self
            .client
            .get("/store-gateway/tenants")
            .send()
            .await
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("get store-gateway tenants request failed: {e}"),
            })?;

        let body = check_response(response).await?;
        serde_json::from_slice(&body).map_err(|e| OperationError::Deserialize {
            target_type: "Value".into(),
            reason: e.to_string(),
        })
    }
}

/// List blocks for a specific tenant in the store-gateway.
///
/// Calls `GET /store-gateway/tenants/{tenant}/blocks`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_mimir::{MimirClient, store_gateway::GetTenantBlocks};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let mimir = MimirClient::from_context(&ctx).await?;
/// let op = GetTenantBlocks::new(mimir, "tenant-1");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetTenantBlocks {
    client: MimirClient,
    tenant: String,
}

impl GetTenantBlocks {
    /// Create a new tenant blocks query.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_mimir::{MimirClient, store_gateway::GetTenantBlocks};
    /// use reqwest::Client;
    ///
    /// let mimir = MimirClient::new("http://mimir:8080", Client::new());
    /// let op = GetTenantBlocks::new(mimir, "tenant-1");
    /// ```
    pub fn new(client: MimirClient, tenant: &str) -> Self {
        Self {
            client,
            tenant: tenant.to_owned(),
        }
    }
}

#[async_trait]
impl Operation for GetTenantBlocks {
    fn kind(&self) -> &str {
        "mimir"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "operation": "store_gateway_get_tenant_blocks",
            "tenant": self.tenant,
        }))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails or the tenant
    /// contains path-traversal characters.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        validate_path_segment(&self.tenant, "tenant", "mimir")?;
        let path = format!("/store-gateway/tenants/{}/blocks", self.tenant);
        let response = self
            .client
            .get(&path)
            .send()
            .await
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("get tenant blocks request failed: {e}"),
            })?;

        let body = check_response(response).await?;
        serde_json::from_slice(&body).map_err(|e| OperationError::Deserialize {
            target_type: "Value".into(),
            reason: e.to_string(),
        })
    }
}
