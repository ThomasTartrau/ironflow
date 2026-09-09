//! Read-only rule query operations.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext};
use serde_json::{Value, json};

use crate::MimirClient;
use crate::error::{check_response, validate_path_segment};

/// List all rule groups across all namespaces.
///
/// Calls `GET /prometheus/api/v1/rules`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_mimir::{MimirClient, rules::GetRules};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let mimir = MimirClient::from_context(&ctx).await?;
/// let op = GetRules::new(mimir);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetRules {
    client: MimirClient,
}

impl GetRules {
    /// Create a new rules listing operation.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_mimir::{MimirClient, rules::GetRules};
    /// use reqwest::Client;
    ///
    /// let mimir = MimirClient::new("http://mimir:8080", Client::new());
    /// let op = GetRules::new(mimir);
    /// ```
    pub fn new(client: MimirClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Operation for GetRules {
    fn kind(&self) -> &str {
        "mimir"
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
            .get("/prometheus/api/v1/rules")
            .send()
            .await
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("get rules request failed: {e}"),
            })?;

        let body = check_response(response).await?;
        serde_json::from_slice(&body).map_err(|e| OperationError::Deserialize {
            target_type: "Value".into(),
            reason: e.to_string(),
        })
    }
}

/// List rule groups for a specific namespace.
///
/// Calls `GET /prometheus/config/v1/rules/{namespace}`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_mimir::{MimirClient, rules::GetRulesByNamespace};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let mimir = MimirClient::from_context(&ctx).await?;
/// let op = GetRulesByNamespace::new(mimir, "production");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetRulesByNamespace {
    client: MimirClient,
    namespace: String,
}

impl GetRulesByNamespace {
    /// Create a new namespace rules query.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_mimir::{MimirClient, rules::GetRulesByNamespace};
    /// use reqwest::Client;
    ///
    /// let mimir = MimirClient::new("http://mimir:8080", Client::new());
    /// let op = GetRulesByNamespace::new(mimir, "production");
    /// ```
    pub fn new(client: MimirClient, namespace: &str) -> Self {
        Self {
            client,
            namespace: namespace.to_owned(),
        }
    }
}

#[async_trait]
impl Operation for GetRulesByNamespace {
    fn kind(&self) -> &str {
        "mimir"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "operation": "get_rules_by_namespace",
            "namespace": self.namespace,
        }))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails, the namespace
    /// contains path-traversal characters, or the response status is not 2xx.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        validate_path_segment(&self.namespace, "namespace", "mimir")?;
        let path = format!("/prometheus/config/v1/rules/{}", self.namespace);
        let response = self
            .client
            .get(&path)
            .send()
            .await
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("get rules by namespace request failed: {e}"),
            })?;

        let body = check_response(response).await?;
        serde_json::from_slice(&body).map_err(|e| OperationError::Deserialize {
            target_type: "Value".into(),
            reason: e.to_string(),
        })
    }
}

/// Retrieve a specific rule group.
///
/// Calls `GET /prometheus/config/v1/rules/{namespace}/{group}`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_mimir::{MimirClient, rules::GetRuleGroup};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let mimir = MimirClient::from_context(&ctx).await?;
/// let op = GetRuleGroup::new(mimir, "production", "cpu-alerts");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetRuleGroup {
    client: MimirClient,
    namespace: String,
    group: String,
}

impl GetRuleGroup {
    /// Create a new rule group query.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_mimir::{MimirClient, rules::GetRuleGroup};
    /// use reqwest::Client;
    ///
    /// let mimir = MimirClient::new("http://mimir:8080", Client::new());
    /// let op = GetRuleGroup::new(mimir, "production", "cpu-alerts");
    /// ```
    pub fn new(client: MimirClient, namespace: &str, group: &str) -> Self {
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
        "mimir"
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
    /// Returns [`OperationError::Http`] if the request fails, a path segment
    /// contains traversal characters, or the response status is not 2xx.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        validate_path_segment(&self.namespace, "namespace", "mimir")?;
        validate_path_segment(&self.group, "group", "mimir")?;
        let path = format!(
            "/prometheus/config/v1/rules/{}/{}",
            self.namespace, self.group
        );
        let response = self
            .client
            .get(&path)
            .send()
            .await
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("get rule group request failed: {e}"),
            })?;

        let body = check_response(response).await?;
        serde_json::from_slice(&body).map_err(|e| OperationError::Deserialize {
            target_type: "Value".into(),
            reason: e.to_string(),
        })
    }
}

/// List all rules across all tenants (admin endpoint).
///
/// Calls `GET /prometheus/api/v1/rules` with the `X-Scope-OrgID` header
/// set to the special `__admin__` tenant.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_mimir::{MimirClient, rules::GetAllTenantRules};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let mimir = MimirClient::from_context(&ctx).await?;
/// let op = GetAllTenantRules::new(mimir);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetAllTenantRules {
    client: MimirClient,
}

impl GetAllTenantRules {
    /// Create a new all-tenants rules query.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_mimir::{MimirClient, rules::GetAllTenantRules};
    /// use reqwest::Client;
    ///
    /// let mimir = MimirClient::new("http://mimir:8080", Client::new());
    /// let op = GetAllTenantRules::new(mimir);
    /// ```
    pub fn new(client: MimirClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Operation for GetAllTenantRules {
    fn kind(&self) -> &str {
        "mimir"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({"operation": "get_all_tenant_rules"}))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails or the response
    /// status is not 2xx.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response = self
            .client
            .get("/prometheus/api/v1/rules")
            .header("X-Scope-OrgID", "__admin__")
            .send()
            .await
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("get all tenant rules request failed: {e}"),
            })?;

        let body = check_response(response).await?;
        serde_json::from_slice(&body).map_err(|e| OperationError::Deserialize {
            target_type: "Value".into(),
            reason: e.to_string(),
        })
    }
}
