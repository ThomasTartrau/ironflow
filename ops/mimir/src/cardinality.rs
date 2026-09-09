//! Cardinality analysis operations.
//!
//! These operations provide insight into label cardinality,
//! useful for identifying high-cardinality labels that increase storage cost.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext};
use serde_json::{Value, json};

use crate::MimirClient;
use crate::error::check_response;

/// Retrieve label names cardinality (Mimir-specific).
///
/// Calls `GET /prometheus/api/v1/cardinality/label_names` with optional
/// selector and limit.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_mimir::{MimirClient, cardinality::GetLabelNamesCardinality};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let mimir = MimirClient::from_context(&ctx).await?;
/// let op = GetLabelNamesCardinality::new(mimir);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetLabelNamesCardinality {
    client: MimirClient,
    selector: Option<String>,
    limit: Option<u64>,
}

impl GetLabelNamesCardinality {
    /// Create a new label names cardinality query.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_mimir::{MimirClient, cardinality::GetLabelNamesCardinality};
    /// use reqwest::Client;
    ///
    /// let mimir = MimirClient::new("http://mimir:8080", Client::new());
    /// let op = GetLabelNamesCardinality::new(mimir);
    /// ```
    pub fn new(client: MimirClient) -> Self {
        Self {
            client,
            selector: None,
            limit: None,
        }
    }

    /// Filter by series selector.
    #[must_use]
    pub fn selector(mut self, selector: &str) -> Self {
        self.selector = Some(selector.to_owned());
        self
    }

    /// Limit the number of results.
    #[must_use]
    pub fn limit(mut self, limit: u64) -> Self {
        self.limit = Some(limit);
        self
    }
}

#[async_trait]
impl Operation for GetLabelNamesCardinality {
    fn kind(&self) -> &str {
        "mimir"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "operation": "get_label_names_cardinality",
            "selector": self.selector,
            "limit": self.limit,
        }))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails or the response
    /// status is not 2xx.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let mut req = self
            .client
            .get("/prometheus/api/v1/cardinality/label_names");
        if let Some(ref s) = self.selector {
            req = req.query(&[("selector", s)]);
        }
        if let Some(l) = self.limit {
            req = req.query(&[("limit", &l.to_string())]);
        }

        let response = req.send().await.map_err(|e| OperationError::Http {
            status: None,
            message: format!("get label names cardinality request failed: {e}"),
        })?;

        let body = check_response(response).await?;
        serde_json::from_slice(&body).map_err(|e| OperationError::Deserialize {
            target_type: "Value".into(),
            reason: e.to_string(),
        })
    }
}

/// Retrieve label values cardinality for a specific label (Mimir-specific).
///
/// Calls `GET /prometheus/api/v1/cardinality/label_values` with a label name
/// and optional selector and limit.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_mimir::{MimirClient, cardinality::GetLabelValuesCardinality};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let mimir = MimirClient::from_context(&ctx).await?;
/// let op = GetLabelValuesCardinality::new(mimir, "__name__");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetLabelValuesCardinality {
    client: MimirClient,
    label_names: String,
    selector: Option<String>,
    limit: Option<u64>,
}

impl GetLabelValuesCardinality {
    /// Create a new label values cardinality query.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_mimir::{MimirClient, cardinality::GetLabelValuesCardinality};
    /// use reqwest::Client;
    ///
    /// let mimir = MimirClient::new("http://mimir:8080", Client::new());
    /// let op = GetLabelValuesCardinality::new(mimir, "__name__");
    /// ```
    pub fn new(client: MimirClient, label_names: &str) -> Self {
        Self {
            client,
            label_names: label_names.to_owned(),
            selector: None,
            limit: None,
        }
    }

    /// Filter by series selector.
    #[must_use]
    pub fn selector(mut self, selector: &str) -> Self {
        self.selector = Some(selector.to_owned());
        self
    }

    /// Limit the number of results.
    #[must_use]
    pub fn limit(mut self, limit: u64) -> Self {
        self.limit = Some(limit);
        self
    }
}

#[async_trait]
impl Operation for GetLabelValuesCardinality {
    fn kind(&self) -> &str {
        "mimir"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "operation": "get_label_values_cardinality",
            "label_names": self.label_names,
            "selector": self.selector,
            "limit": self.limit,
        }))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails or the response
    /// status is not 2xx.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let mut req = self
            .client
            .get("/prometheus/api/v1/cardinality/label_values")
            .query(&[("label_names[]", &self.label_names)]);
        if let Some(ref s) = self.selector {
            req = req.query(&[("selector", s)]);
        }
        if let Some(l) = self.limit {
            req = req.query(&[("limit", &l.to_string())]);
        }

        let response = req.send().await.map_err(|e| OperationError::Http {
            status: None,
            message: format!("get label values cardinality request failed: {e}"),
        })?;

        let body = check_response(response).await?;
        serde_json::from_slice(&body).map_err(|e| OperationError::Deserialize {
            target_type: "Value".into(),
            reason: e.to_string(),
        })
    }
}
