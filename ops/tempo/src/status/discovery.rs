//! Service and endpoint discovery, plus runtime configuration.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext};
use serde_json::{Value, json};

use crate::TempoClient;
use crate::error::{check_response, parse_json_body, send_request};

/// Retrieve the list of Tempo service components.
///
/// Calls `GET /status/services`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_tempo::{TempoClient, status::GetServices};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let tempo = TempoClient::from_context(&ctx).await?;
/// let result = GetServices::new(tempo).execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetServices {
    client: TempoClient,
}

impl GetServices {
    /// Create a new services listing operation.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_tempo::{TempoClient, status::GetServices};
    /// use reqwest::Client;
    ///
    /// let tempo = TempoClient::new("http://tempo:3200", Client::new());
    /// let op = GetServices::new(tempo);
    /// ```
    pub fn new(client: TempoClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Operation for GetServices {
    fn kind(&self) -> &str {
        "tempo"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({"operation": "get_services"}))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response = send_request(self.client.get("/status/services"), "get services").await?;
        let body = check_response(response).await?;
        parse_json_body(&body)
    }
}

/// Retrieve the list of Tempo HTTP endpoints.
///
/// Calls `GET /status/endpoints`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_tempo::{TempoClient, status::GetEndpoints};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let tempo = TempoClient::from_context(&ctx).await?;
/// let result = GetEndpoints::new(tempo).execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetEndpoints {
    client: TempoClient,
}

impl GetEndpoints {
    /// Create a new endpoints listing operation.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_tempo::{TempoClient, status::GetEndpoints};
    /// use reqwest::Client;
    ///
    /// let tempo = TempoClient::new("http://tempo:3200", Client::new());
    /// let op = GetEndpoints::new(tempo);
    /// ```
    pub fn new(client: TempoClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Operation for GetEndpoints {
    fn kind(&self) -> &str {
        "tempo"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({"operation": "get_endpoints"}))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response = send_request(self.client.get("/status/endpoints"), "get endpoints").await?;
        let body = check_response(response).await?;
        parse_json_body(&body)
    }
}

/// Retrieve Tempo's runtime configuration.
///
/// Calls `GET /status/config`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_tempo::{TempoClient, status::GetConfig};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let tempo = TempoClient::from_context(&ctx).await?;
/// let result = GetConfig::new(tempo).execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetConfig {
    client: TempoClient,
}

impl GetConfig {
    /// Create a new config query.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_tempo::{TempoClient, status::GetConfig};
    /// use reqwest::Client;
    ///
    /// let tempo = TempoClient::new("http://tempo:3200", Client::new());
    /// let op = GetConfig::new(tempo);
    /// ```
    pub fn new(client: TempoClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Operation for GetConfig {
    fn kind(&self) -> &str {
        "tempo"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({"operation": "get_config"}))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response = send_request(self.client.get("/status/config"), "get config").await?;

        let body = check_response(response).await?;
        let text = String::from_utf8_lossy(&body);
        Ok(json!({"config": text.as_ref()}))
    }
}
