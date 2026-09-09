//! Read-only rule operations: list rules, get a group, list alerts.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext};
use serde_json::{Value, from_slice, json};

use crate::LokiClient;
use crate::error::{check_response, validate_path_segment};

/// Retrieve all rules across all namespaces.
///
/// Calls `GET /loki/api/v1/rules`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_loki::{LokiClient, rules::GetRules};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let loki = LokiClient::from_context(&ctx).await?;
/// let op = GetRules::new(loki);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetRules {
    client: LokiClient,
}

impl GetRules {
    /// Create a new operation to list all rules.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_loki::{LokiClient, rules::GetRules};
    /// use reqwest::Client;
    ///
    /// let loki = LokiClient::new("http://loki:3100", Client::new());
    /// let op = GetRules::new(loki);
    /// ```
    pub fn new(client: LokiClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Operation for GetRules {
    fn kind(&self) -> &str {
        "loki"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({"operation": "get_rules"}))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails or the response
    /// status is not 2xx.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response = self
            .client
            .get("/loki/api/v1/rules")
            .send()
            .await
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("get rules request failed: {e}"),
            })?;

        let body = check_response(response).await?;
        from_slice(&body).map_err(|e| OperationError::Deserialize {
            target_type: "Value".into(),
            reason: e.to_string(),
        })
    }
}

/// Retrieve rules for a specific namespace.
///
/// Calls `GET /loki/api/v1/rules/{namespace}`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_loki::{LokiClient, rules::GetRulesByNamespace};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let loki = LokiClient::from_context(&ctx).await?;
/// let op = GetRulesByNamespace::new(loki, "production");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetRulesByNamespace {
    client: LokiClient,
    namespace: String,
}

impl GetRulesByNamespace {
    /// Create a new operation to list rules in a namespace.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_loki::{LokiClient, rules::GetRulesByNamespace};
    /// use reqwest::Client;
    ///
    /// let loki = LokiClient::new("http://loki:3100", Client::new());
    /// let op = GetRulesByNamespace::new(loki, "production");
    /// ```
    pub fn new(client: LokiClient, namespace: &str) -> Self {
        Self {
            client,
            namespace: namespace.to_owned(),
        }
    }
}

#[async_trait]
impl Operation for GetRulesByNamespace {
    fn kind(&self) -> &str {
        "loki"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "operation": "get_rules_by_namespace",
            "namespace": self.namespace,
        }))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails or the response
    /// status is not 2xx.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        validate_path_segment(&self.namespace, "namespace")?;
        let response = self
            .client
            .get(&format!("/loki/api/v1/rules/{}", self.namespace))
            .send()
            .await
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("get rules by namespace request failed: {e}"),
            })?;

        let body = check_response(response).await?;
        from_slice(&body).map_err(|e| OperationError::Deserialize {
            target_type: "Value".into(),
            reason: e.to_string(),
        })
    }
}

/// Retrieve a specific rule group.
///
/// Calls `GET /loki/api/v1/rules/{namespace}/{group}`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_loki::{LokiClient, rules::GetRuleGroup};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let loki = LokiClient::from_context(&ctx).await?;
/// let op = GetRuleGroup::new(loki, "production", "high-error-rate");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetRuleGroup {
    client: LokiClient,
    namespace: String,
    group: String,
}

impl GetRuleGroup {
    /// Create a new operation to get a rule group.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_loki::{LokiClient, rules::GetRuleGroup};
    /// use reqwest::Client;
    ///
    /// let loki = LokiClient::new("http://loki:3100", Client::new());
    /// let op = GetRuleGroup::new(loki, "production", "high-error-rate");
    /// ```
    pub fn new(client: LokiClient, namespace: &str, group: &str) -> Self {
        Self {
            client,
            namespace: namespace.to_owned(),
            group: group.to_owned(),
        }
    }
}

#[async_trait]
impl Operation for GetRuleGroup {
    fn kind(&self) -> &str {
        "loki"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "operation": "get_rule_group",
            "namespace": self.namespace,
            "group": self.group,
        }))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails or the response
    /// status is not 2xx.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        validate_path_segment(&self.namespace, "namespace")?;
        validate_path_segment(&self.group, "group")?;
        let response = self
            .client
            .get(&format!(
                "/loki/api/v1/rules/{}/{}",
                self.namespace, self.group
            ))
            .send()
            .await
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("get rule group request failed: {e}"),
            })?;

        let body = check_response(response).await?;
        from_slice(&body).map_err(|e| OperationError::Deserialize {
            target_type: "Value".into(),
            reason: e.to_string(),
        })
    }
}

/// Retrieve all active alerts.
///
/// Calls `GET /loki/api/v1/alerts`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_loki::{LokiClient, rules::GetAlerts};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let loki = LokiClient::from_context(&ctx).await?;
/// let op = GetAlerts::new(loki);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetAlerts {
    client: LokiClient,
}

impl GetAlerts {
    /// Create a new operation to list active alerts.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_loki::{LokiClient, rules::GetAlerts};
    /// use reqwest::Client;
    ///
    /// let loki = LokiClient::new("http://loki:3100", Client::new());
    /// let op = GetAlerts::new(loki);
    /// ```
    pub fn new(client: LokiClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Operation for GetAlerts {
    fn kind(&self) -> &str {
        "loki"
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
            .get("/loki/api/v1/alerts")
            .send()
            .await
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("get alerts request failed: {e}"),
            })?;

        let body = check_response(response).await?;
        from_slice(&body).map_err(|e| OperationError::Deserialize {
            target_type: "Value".into(),
            reason: e.to_string(),
        })
    }
}
