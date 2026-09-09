//! Label name and value discovery operations.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext};
use serde_json::{Value, json};

use crate::MimirClient;
use crate::error::{check_response, validate_path_segment};

/// Retrieve the list of known label names.
///
/// Calls `GET /prometheus/api/v1/labels` with optional start/end timestamps.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_mimir::{MimirClient, series::GetLabels};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let mimir = MimirClient::from_context(&ctx).await?;
/// let op = GetLabels::new(mimir);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetLabels {
    client: MimirClient,
    start: Option<String>,
    end: Option<String>,
}

impl GetLabels {
    /// Create a new label listing operation.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_mimir::{MimirClient, series::GetLabels};
    /// use reqwest::Client;
    ///
    /// let mimir = MimirClient::new("http://mimir:8080", Client::new());
    /// let op = GetLabels::new(mimir);
    /// ```
    pub fn new(client: MimirClient) -> Self {
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
        "mimir"
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
        let mut req = self.client.get("/prometheus/api/v1/labels");
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
        serde_json::from_slice(&body).map_err(|e| OperationError::Deserialize {
            target_type: "Value".into(),
            reason: e.to_string(),
        })
    }
}

/// Retrieve known values for a given label name.
///
/// Calls `GET /prometheus/api/v1/label/{name}/values`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_mimir::{MimirClient, series::GetLabelValues};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let mimir = MimirClient::from_context(&ctx).await?;
/// let op = GetLabelValues::new(mimir, "job");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetLabelValues {
    client: MimirClient,
    label_name: String,
    start: Option<String>,
    end: Option<String>,
}

impl GetLabelValues {
    /// Create a new label values query.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_mimir::{MimirClient, series::GetLabelValues};
    /// use reqwest::Client;
    ///
    /// let mimir = MimirClient::new("http://mimir:8080", Client::new());
    /// let op = GetLabelValues::new(mimir, "job");
    /// ```
    pub fn new(client: MimirClient, label_name: &str) -> Self {
        Self {
            client,
            label_name: label_name.to_owned(),
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
        "mimir"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "operation": "get_label_values",
            "label_name": self.label_name,
            "start": self.start,
            "end": self.end,
        }))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails, the label name
    /// contains path-traversal characters, or the response status is not 2xx.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        validate_path_segment(&self.label_name, "label_name", "mimir")?;
        let path = format!("/prometheus/api/v1/label/{}/values", self.label_name);
        let mut req = self.client.get(&path);
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
        serde_json::from_slice(&body).map_err(|e| OperationError::Deserialize {
            target_type: "Value".into(),
            reason: e.to_string(),
        })
    }
}
