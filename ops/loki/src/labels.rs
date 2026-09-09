//! Label and series discovery operations.
//!
//! These operations query the Loki label and series metadata endpoints,
//! useful for building dynamic queries and understanding the shape of
//! ingested data.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext};
use serde_json::{Value, from_slice, json};

use crate::LokiClient;
use crate::error::{check_response, validate_path_segment};

/// Retrieve the list of known label names.
///
/// Calls `GET /loki/api/v1/labels` with optional start/end timestamps.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_loki::{LokiClient, labels::GetLabels};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let loki = LokiClient::from_context(&ctx).await?;
/// let op = GetLabels::new(loki);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetLabels {
    client: LokiClient,
    start: Option<String>,
    end: Option<String>,
}

impl GetLabels {
    /// Create a new label listing operation.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_loki::{LokiClient, labels::GetLabels};
    /// use reqwest::Client;
    ///
    /// let loki = LokiClient::new("http://loki:3100", Client::new());
    /// let op = GetLabels::new(loki);
    /// ```
    pub fn new(client: LokiClient) -> Self {
        Self {
            client,
            start: None,
            end: None,
        }
    }

    /// Restrict results to labels seen after this timestamp.
    #[must_use]
    pub fn start(mut self, start: &str) -> Self {
        self.start = Some(start.to_owned());
        self
    }

    /// Restrict results to labels seen before this timestamp.
    #[must_use]
    pub fn end(mut self, end: &str) -> Self {
        self.end = Some(end.to_owned());
        self
    }
}

#[async_trait]
impl Operation for GetLabels {
    fn kind(&self) -> &str {
        "loki"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "operation": "get_labels",
            "start": self.start,
            "end": self.end,
        }))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails or the response
    /// status is not 2xx.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let mut req = self.client.get("/loki/api/v1/labels");
        if let Some(ref s) = self.start {
            req = req.query(&[("start", s)]);
        }
        if let Some(ref e) = self.end {
            req = req.query(&[("end", e)]);
        }

        let response = req.send().await.map_err(|e| OperationError::Http {
            status: None,
            message: format!("get labels request failed: {e}"),
        })?;

        let body = check_response(response).await?;
        from_slice(&body).map_err(|e| OperationError::Deserialize {
            target_type: "Value".into(),
            reason: e.to_string(),
        })
    }
}

/// Retrieve the known values for a given label name.
///
/// Calls `GET /loki/api/v1/label/{name}/values` with optional time range.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_loki::{LokiClient, labels::GetLabelValues};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let loki = LokiClient::from_context(&ctx).await?;
/// let op = GetLabelValues::new(loki, "job");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetLabelValues {
    client: LokiClient,
    name: String,
    start: Option<String>,
    end: Option<String>,
}

impl GetLabelValues {
    /// Create a new label values query.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_loki::{LokiClient, labels::GetLabelValues};
    /// use reqwest::Client;
    ///
    /// let loki = LokiClient::new("http://loki:3100", Client::new());
    /// let op = GetLabelValues::new(loki, "job");
    /// ```
    pub fn new(client: LokiClient, name: &str) -> Self {
        Self {
            client,
            name: name.to_owned(),
            start: None,
            end: None,
        }
    }

    /// Restrict results to values seen after this timestamp.
    #[must_use]
    pub fn start(mut self, start: &str) -> Self {
        self.start = Some(start.to_owned());
        self
    }

    /// Restrict results to values seen before this timestamp.
    #[must_use]
    pub fn end(mut self, end: &str) -> Self {
        self.end = Some(end.to_owned());
        self
    }
}

#[async_trait]
impl Operation for GetLabelValues {
    fn kind(&self) -> &str {
        "loki"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "operation": "get_label_values",
            "name": self.name,
            "start": self.start,
            "end": self.end,
        }))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails or the response
    /// status is not 2xx.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        validate_path_segment(&self.name, "label name")?;
        let mut req = self
            .client
            .get(&format!("/loki/api/v1/label/{}/values", self.name));
        if let Some(ref s) = self.start {
            req = req.query(&[("start", s)]);
        }
        if let Some(ref e) = self.end {
            req = req.query(&[("end", e)]);
        }

        let response = req.send().await.map_err(|e| OperationError::Http {
            status: None,
            message: format!("get label values request failed: {e}"),
        })?;

        let body = check_response(response).await?;
        from_slice(&body).map_err(|e| OperationError::Deserialize {
            target_type: "Value".into(),
            reason: e.to_string(),
        })
    }
}

/// Retrieve the list of time series matching a set of label matchers.
///
/// Calls `GET /loki/api/v1/series` with one or more `match[]` parameters.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_loki::{LokiClient, labels::GetSeries};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let loki = LokiClient::from_context(&ctx).await?;
/// let op = GetSeries::new(loki, vec![r#"{job="varlogs"}"#.into()]);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetSeries {
    client: LokiClient,
    matchers: Vec<String>,
    start: Option<String>,
    end: Option<String>,
}

impl GetSeries {
    /// Create a new series query.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_loki::{LokiClient, labels::GetSeries};
    /// use reqwest::Client;
    ///
    /// let loki = LokiClient::new("http://loki:3100", Client::new());
    /// let op = GetSeries::new(loki, vec![r#"{job="varlogs"}"#.into()]);
    /// ```
    pub fn new(client: LokiClient, matchers: Vec<String>) -> Self {
        Self {
            client,
            matchers,
            start: None,
            end: None,
        }
    }

    /// Restrict results to series seen after this timestamp.
    #[must_use]
    pub fn start(mut self, start: &str) -> Self {
        self.start = Some(start.to_owned());
        self
    }

    /// Restrict results to series seen before this timestamp.
    #[must_use]
    pub fn end(mut self, end: &str) -> Self {
        self.end = Some(end.to_owned());
        self
    }
}

#[async_trait]
impl Operation for GetSeries {
    fn kind(&self) -> &str {
        "loki"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "operation": "get_series",
            "matchers": self.matchers,
            "start": self.start,
            "end": self.end,
        }))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails or the response
    /// status is not 2xx.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let mut pairs: Vec<(&str, &str)> = Vec::new();
        for m in &self.matchers {
            pairs.push(("match[]", m.as_str()));
        }

        let mut req = self.client.get("/loki/api/v1/series").query(&pairs);
        if let Some(ref s) = self.start {
            req = req.query(&[("start", s)]);
        }
        if let Some(ref e) = self.end {
            req = req.query(&[("end", e)]);
        }

        let response = req.send().await.map_err(|e| OperationError::Http {
            status: None,
            message: format!("get series request failed: {e}"),
        })?;

        let body = check_response(response).await?;
        from_slice(&body).map_err(|e| OperationError::Deserialize {
            target_type: "Value".into(),
            reason: e.to_string(),
        })
    }
}
