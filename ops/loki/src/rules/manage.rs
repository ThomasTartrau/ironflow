//! Write operations for rule groups and namespaces.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext};
use serde_json::{Value, from_slice, json};

use crate::LokiClient;
use crate::error::{check_response, validate_path_segment};

/// Create or update a rule group.
///
/// Calls `POST /loki/api/v1/rules/{namespace}` with the rule group YAML body.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_loki::{LokiClient, rules::CreateRuleGroup};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let loki = LokiClient::from_context(&ctx).await?;
/// let yaml_body = "name: test-group\ninterval: 1m\nrules: []";
/// let op = CreateRuleGroup::new(loki, "production", yaml_body);
/// op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct CreateRuleGroup {
    client: LokiClient,
    namespace: String,
    body: String,
}

impl CreateRuleGroup {
    /// Create a new rule group creation operation.
    ///
    /// The `body` must be a valid YAML rule group definition.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_loki::{LokiClient, rules::CreateRuleGroup};
    /// use reqwest::Client;
    ///
    /// let loki = LokiClient::new("http://loki:3100", Client::new());
    /// let op = CreateRuleGroup::new(loki, "production", "name: group\nrules: []");
    /// ```
    pub fn new(client: LokiClient, namespace: &str, body: &str) -> Self {
        Self {
            client,
            namespace: namespace.to_owned(),
            body: body.to_owned(),
        }
    }
}

#[async_trait]
impl Operation for CreateRuleGroup {
    fn kind(&self) -> &str {
        "loki"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "operation": "create_rule_group",
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
            .post(&format!("/loki/api/v1/rules/{}", self.namespace))
            .header("Content-Type", "application/yaml")
            .body(self.body.clone())
            .send()
            .await
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("create rule group request failed: {e}"),
            })?;

        let body = check_response(response).await?;
        if body.is_empty() {
            return Ok(json!({"status": "success"}));
        }
        from_slice(&body).map_err(|e| OperationError::Deserialize {
            target_type: "Value".into(),
            reason: e.to_string(),
        })
    }
}

/// Delete a rule group.
///
/// Calls `DELETE /loki/api/v1/rules/{namespace}/{group}`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_loki::{LokiClient, rules::DeleteRuleGroup};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let loki = LokiClient::from_context(&ctx).await?;
/// let op = DeleteRuleGroup::new(loki, "production", "old-group");
/// op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct DeleteRuleGroup {
    client: LokiClient,
    namespace: String,
    group: String,
}

impl DeleteRuleGroup {
    /// Create a new rule group deletion operation.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_loki::{LokiClient, rules::DeleteRuleGroup};
    /// use reqwest::Client;
    ///
    /// let loki = LokiClient::new("http://loki:3100", Client::new());
    /// let op = DeleteRuleGroup::new(loki, "production", "old-group");
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
impl Operation for DeleteRuleGroup {
    fn kind(&self) -> &str {
        "loki"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "operation": "delete_rule_group",
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
            .delete(&format!(
                "/loki/api/v1/rules/{}/{}",
                self.namespace, self.group
            ))
            .send()
            .await
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("delete rule group request failed: {e}"),
            })?;

        let body = check_response(response).await?;
        if body.is_empty() {
            return Ok(json!({"status": "success"}));
        }
        from_slice(&body).map_err(|e| OperationError::Deserialize {
            target_type: "Value".into(),
            reason: e.to_string(),
        })
    }
}

/// Delete all rule groups in a namespace.
///
/// Calls `DELETE /loki/api/v1/rules/{namespace}`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_loki::{LokiClient, rules::DeleteRuleNamespace};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let loki = LokiClient::from_context(&ctx).await?;
/// let op = DeleteRuleNamespace::new(loki, "deprecated");
/// op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct DeleteRuleNamespace {
    client: LokiClient,
    namespace: String,
}

impl DeleteRuleNamespace {
    /// Create a new namespace deletion operation.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_loki::{LokiClient, rules::DeleteRuleNamespace};
    /// use reqwest::Client;
    ///
    /// let loki = LokiClient::new("http://loki:3100", Client::new());
    /// let op = DeleteRuleNamespace::new(loki, "deprecated");
    /// ```
    pub fn new(client: LokiClient, namespace: &str) -> Self {
        Self {
            client,
            namespace: namespace.to_owned(),
        }
    }
}

#[async_trait]
impl Operation for DeleteRuleNamespace {
    fn kind(&self) -> &str {
        "loki"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "operation": "delete_rule_namespace",
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
            .delete(&format!("/loki/api/v1/rules/{}", self.namespace))
            .send()
            .await
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("delete rule namespace request failed: {e}"),
            })?;

        let body = check_response(response).await?;
        if body.is_empty() {
            return Ok(json!({"status": "success"}));
        }
        from_slice(&body).map_err(|e| OperationError::Deserialize {
            target_type: "Value".into(),
            reason: e.to_string(),
        })
    }
}
