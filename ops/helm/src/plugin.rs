//! Helm plugin operations: install, uninstall, list, update.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde_json::Value;

use crate::client::HelmClient;
use crate::helpers::{run_helm, to_value};
use crate::util::TextOutput;

/// Install a Helm plugin.
///
/// Wraps `helm plugin install <path|url>`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_helm::HelmClient;
/// use ironflow_ops_helm::plugin::PluginInstall;
/// use ironflow_core::operation::Operation;
///
/// let client = HelmClient::default();
/// let op = PluginInstall::new(client, "https://github.com/example/helm-plugin");
/// assert_eq!(op.kind(), "helm");
/// ```
pub struct PluginInstall {
    client: HelmClient,
    source: String,
    version: Option<String>,
}

impl PluginInstall {
    /// Create a new plugin-install operation.
    pub fn new(client: HelmClient, source: impl Into<String>) -> Self {
        Self {
            client,
            source: source.into(),
            version: None,
        }
    }

    /// Install a specific version.
    pub fn version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    /// Execute and return the output.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Shell`] if the command fails.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<TextOutput, OperationError> {
        let mut args = vec!["plugin", "install", &self.source];
        let ver;
        if let Some(ref v) = self.version {
            ver = v.clone();
            args.push("--version");
            args.push(&ver);
        }
        let stdout = run_helm(&self.client, &args).await?;
        Ok(TextOutput {
            output: stdout.trim().to_string(),
        })
    }
}

#[async_trait]
impl Operation for PluginInstall {
    fn kind(&self) -> &str {
        "helm"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "command": "plugin install",
            "source": self.source,
        }))
    }
}

impl TypedOperation for PluginInstall {
    type Output = TextOutput;
}

/// Uninstall a Helm plugin.
///
/// Wraps `helm plugin uninstall <name>`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_helm::HelmClient;
/// use ironflow_ops_helm::plugin::PluginUninstall;
/// use ironflow_core::operation::Operation;
///
/// let client = HelmClient::default();
/// let op = PluginUninstall::new(client, "my-plugin");
/// assert_eq!(op.kind(), "helm");
/// ```
pub struct PluginUninstall {
    client: HelmClient,
    name: String,
}

impl PluginUninstall {
    /// Create a new plugin-uninstall operation.
    pub fn new(client: HelmClient, name: impl Into<String>) -> Self {
        Self {
            client,
            name: name.into(),
        }
    }

    /// Execute and return the output.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Shell`] if the command fails.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<TextOutput, OperationError> {
        let stdout = run_helm(&self.client, &["plugin", "uninstall", &self.name]).await?;
        Ok(TextOutput {
            output: stdout.trim().to_string(),
        })
    }
}

#[async_trait]
impl Operation for PluginUninstall {
    fn kind(&self) -> &str {
        "helm"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "command": "plugin uninstall",
            "name": self.name,
        }))
    }
}

impl TypedOperation for PluginUninstall {
    type Output = TextOutput;
}

/// List installed Helm plugins.
///
/// Wraps `helm plugin list`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_helm::HelmClient;
/// use ironflow_ops_helm::plugin::PluginList;
/// use ironflow_core::operation::Operation;
///
/// let client = HelmClient::default();
/// let op = PluginList::new(client);
/// assert_eq!(op.kind(), "helm");
/// ```
pub struct PluginList {
    client: HelmClient,
}

impl PluginList {
    /// Create a new plugin-list operation.
    pub fn new(client: HelmClient) -> Self {
        Self { client }
    }

    /// Execute and return the output.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Shell`] if the command fails.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<TextOutput, OperationError> {
        let stdout = run_helm(&self.client, &["plugin", "list"]).await?;
        Ok(TextOutput {
            output: stdout.trim().to_string(),
        })
    }
}

#[async_trait]
impl Operation for PluginList {
    fn kind(&self) -> &str {
        "helm"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "command": "plugin list" }))
    }
}

impl TypedOperation for PluginList {
    type Output = TextOutput;
}

/// Update a Helm plugin.
///
/// Wraps `helm plugin update <name>`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_helm::HelmClient;
/// use ironflow_ops_helm::plugin::PluginUpdate;
/// use ironflow_core::operation::Operation;
///
/// let client = HelmClient::default();
/// let op = PluginUpdate::new(client, "my-plugin");
/// assert_eq!(op.kind(), "helm");
/// ```
pub struct PluginUpdate {
    client: HelmClient,
    name: String,
}

impl PluginUpdate {
    /// Create a new plugin-update operation.
    pub fn new(client: HelmClient, name: impl Into<String>) -> Self {
        Self {
            client,
            name: name.into(),
        }
    }

    /// Execute and return the output.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Shell`] if the command fails.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<TextOutput, OperationError> {
        let stdout = run_helm(&self.client, &["plugin", "update", &self.name]).await?;
        Ok(TextOutput {
            output: stdout.trim().to_string(),
        })
    }
}

#[async_trait]
impl Operation for PluginUpdate {
    fn kind(&self) -> &str {
        "helm"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "command": "plugin update",
            "name": self.name,
        }))
    }
}

impl TypedOperation for PluginUpdate {
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
    fn plugin_install_kind() {
        assert_eq!(PluginInstall::new(client(), "url").kind(), "helm");
    }

    #[test]
    fn plugin_uninstall_kind() {
        assert_eq!(PluginUninstall::new(client(), "name").kind(), "helm");
    }

    #[test]
    fn plugin_list_kind() {
        assert_eq!(PluginList::new(client()).kind(), "helm");
    }

    #[test]
    fn plugin_update_kind() {
        assert_eq!(PluginUpdate::new(client(), "name").kind(), "helm");
    }
}
