//! Hash ring status operations.
//!
//! These operations query the state of Loki's internal hash rings used
//! by the distributed components (distributor, index gateway, ruler, compactor).

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext};
use serde_json::Value;
use serde_json::json;

use crate::LokiClient;
use crate::error::check_response;

/// Retrieve the distributor hash ring.
///
/// Calls `GET /distributor/ring`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_loki::{LokiClient, rings::GetDistributorRing};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let loki = LokiClient::from_context(&ctx).await?;
/// let op = GetDistributorRing::new(loki);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetDistributorRing {
    client: LokiClient,
}

impl GetDistributorRing {
    /// Create a new distributor ring query.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_loki::{LokiClient, rings::GetDistributorRing};
    /// use reqwest::Client;
    ///
    /// let loki = LokiClient::new("http://loki:3100", Client::new());
    /// let op = GetDistributorRing::new(loki);
    /// ```
    pub fn new(client: LokiClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Operation for GetDistributorRing {
    fn kind(&self) -> &str {
        "loki"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({"operation": "get_distributor_ring"}))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response = self
            .client
            .get("/distributor/ring")
            .send()
            .await
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("get distributor ring request failed: {e}"),
            })?;

        let body = check_response(response).await?;
        let text = String::from_utf8_lossy(&body);
        Ok(json!({"ring": text.as_ref()}))
    }
}

/// Retrieve the index gateway hash ring.
///
/// Calls `GET /indexgateway/ring`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_loki::{LokiClient, rings::GetIndexGatewayRing};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let loki = LokiClient::from_context(&ctx).await?;
/// let op = GetIndexGatewayRing::new(loki);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetIndexGatewayRing {
    client: LokiClient,
}

impl GetIndexGatewayRing {
    /// Create a new index gateway ring query.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_loki::{LokiClient, rings::GetIndexGatewayRing};
    /// use reqwest::Client;
    ///
    /// let loki = LokiClient::new("http://loki:3100", Client::new());
    /// let op = GetIndexGatewayRing::new(loki);
    /// ```
    pub fn new(client: LokiClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Operation for GetIndexGatewayRing {
    fn kind(&self) -> &str {
        "loki"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({"operation": "get_index_gateway_ring"}))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response = self
            .client
            .get("/indexgateway/ring")
            .send()
            .await
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("get index gateway ring request failed: {e}"),
            })?;

        let body = check_response(response).await?;
        let text = String::from_utf8_lossy(&body);
        Ok(json!({"ring": text.as_ref()}))
    }
}

/// Retrieve the ruler hash ring.
///
/// Calls `GET /ruler/ring`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_loki::{LokiClient, rings::GetRulerRing};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let loki = LokiClient::from_context(&ctx).await?;
/// let op = GetRulerRing::new(loki);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetRulerRing {
    client: LokiClient,
}

impl GetRulerRing {
    /// Create a new ruler ring query.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_loki::{LokiClient, rings::GetRulerRing};
    /// use reqwest::Client;
    ///
    /// let loki = LokiClient::new("http://loki:3100", Client::new());
    /// let op = GetRulerRing::new(loki);
    /// ```
    pub fn new(client: LokiClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Operation for GetRulerRing {
    fn kind(&self) -> &str {
        "loki"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({"operation": "get_ruler_ring"}))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response =
            self.client
                .get("/ruler/ring")
                .send()
                .await
                .map_err(|e| OperationError::Http {
                    status: None,
                    message: format!("get ruler ring request failed: {e}"),
                })?;

        let body = check_response(response).await?;
        let text = String::from_utf8_lossy(&body);
        Ok(json!({"ring": text.as_ref()}))
    }
}

/// Retrieve the compactor hash ring.
///
/// Calls `GET /compactor/ring`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_loki::{LokiClient, rings::GetCompactorRing};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let loki = LokiClient::from_context(&ctx).await?;
/// let op = GetCompactorRing::new(loki);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetCompactorRing {
    client: LokiClient,
}

impl GetCompactorRing {
    /// Create a new compactor ring query.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_loki::{LokiClient, rings::GetCompactorRing};
    /// use reqwest::Client;
    ///
    /// let loki = LokiClient::new("http://loki:3100", Client::new());
    /// let op = GetCompactorRing::new(loki);
    /// ```
    pub fn new(client: LokiClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Operation for GetCompactorRing {
    fn kind(&self) -> &str {
        "loki"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({"operation": "get_compactor_ring"}))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response = self
            .client
            .get("/compactor/ring")
            .send()
            .await
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("get compactor ring request failed: {e}"),
            })?;

        let body = check_response(response).await?;
        let text = String::from_utf8_lossy(&body);
        Ok(json!({"ring": text.as_ref()}))
    }
}
