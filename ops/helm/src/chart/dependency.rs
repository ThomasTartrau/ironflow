//! Chart dependency operations: update, build, list.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde_json::Value;

use crate::client::HelmClient;
use crate::helpers::{run_helm, to_value};
use crate::util::TextOutput;

/// Update chart dependencies.
///
/// Wraps `helm dependency update <chart>`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_helm::HelmClient;
/// use ironflow_ops_helm::chart::DependencyUpdate;
/// use ironflow_core::operation::Operation;
///
/// let client = HelmClient::default();
/// let op = DependencyUpdate::new(client, "./my-chart");
/// assert_eq!(op.kind(), "helm");
/// ```
pub struct DependencyUpdate {
    client: HelmClient,
    chart: String,
}

impl DependencyUpdate {
    /// Create a new dependency-update operation.
    pub fn new(client: HelmClient, chart: impl Into<String>) -> Self {
        Self {
            client,
            chart: chart.into(),
        }
    }

    /// Execute and return the output.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Shell`] if the command fails.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<TextOutput, OperationError> {
        let stdout = run_helm(&self.client, &["dependency", "update", &self.chart]).await?;
        Ok(TextOutput {
            output: stdout.trim().to_string(),
        })
    }
}

#[async_trait]
impl Operation for DependencyUpdate {
    fn kind(&self) -> &str {
        "helm"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "command": "dependency update",
            "chart": self.chart,
        }))
    }
}

impl TypedOperation for DependencyUpdate {
    type Output = TextOutput;
}

/// Build chart dependencies.
///
/// Wraps `helm dependency build <chart>`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_helm::HelmClient;
/// use ironflow_ops_helm::chart::DependencyBuild;
/// use ironflow_core::operation::Operation;
///
/// let client = HelmClient::default();
/// let op = DependencyBuild::new(client, "./my-chart");
/// assert_eq!(op.kind(), "helm");
/// ```
pub struct DependencyBuild {
    client: HelmClient,
    chart: String,
}

impl DependencyBuild {
    /// Create a new dependency-build operation.
    pub fn new(client: HelmClient, chart: impl Into<String>) -> Self {
        Self {
            client,
            chart: chart.into(),
        }
    }

    /// Execute and return the output.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Shell`] if the command fails.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<TextOutput, OperationError> {
        let stdout = run_helm(&self.client, &["dependency", "build", &self.chart]).await?;
        Ok(TextOutput {
            output: stdout.trim().to_string(),
        })
    }
}

#[async_trait]
impl Operation for DependencyBuild {
    fn kind(&self) -> &str {
        "helm"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "command": "dependency build",
            "chart": self.chart,
        }))
    }
}

impl TypedOperation for DependencyBuild {
    type Output = TextOutput;
}

/// List chart dependencies.
///
/// Wraps `helm dependency list <chart>`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_helm::HelmClient;
/// use ironflow_ops_helm::chart::DependencyList;
/// use ironflow_core::operation::Operation;
///
/// let client = HelmClient::default();
/// let op = DependencyList::new(client, "./my-chart");
/// assert_eq!(op.kind(), "helm");
/// ```
pub struct DependencyList {
    client: HelmClient,
    chart: String,
}

impl DependencyList {
    /// Create a new dependency-list operation.
    pub fn new(client: HelmClient, chart: impl Into<String>) -> Self {
        Self {
            client,
            chart: chart.into(),
        }
    }

    /// Execute and return the output.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Shell`] if the command fails.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<TextOutput, OperationError> {
        let stdout = run_helm(&self.client, &["dependency", "list", &self.chart]).await?;
        Ok(TextOutput {
            output: stdout.trim().to_string(),
        })
    }
}

#[async_trait]
impl Operation for DependencyList {
    fn kind(&self) -> &str {
        "helm"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "command": "dependency list",
            "chart": self.chart,
        }))
    }
}

impl TypedOperation for DependencyList {
    type Output = TextOutput;
}

#[cfg(test)]
mod tests {
    use ironflow_core::operation::Operation;

    use super::*;

    fn client() -> HelmClient {
        HelmClient::default()
    }

    #[test]
    fn dependency_update_kind() {
        assert_eq!(DependencyUpdate::new(client(), "./chart").kind(), "helm");
    }

    #[test]
    fn dependency_build_kind() {
        assert_eq!(DependencyBuild::new(client(), "./chart").kind(), "helm");
    }

    #[test]
    fn dependency_list_kind() {
        assert_eq!(DependencyList::new(client(), "./chart").kind(), "helm");
    }
}
