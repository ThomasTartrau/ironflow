//! `helm uninstall` and `helm rollback` operations.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde_json::Value;

use crate::client::HelmClient;
use crate::helpers::{run_helm_with_extra, to_value};
use crate::util::TextOutput;

/// Uninstall a Helm release.
///
/// Wraps `helm uninstall <name>`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_helm::HelmClient;
/// use ironflow_ops_helm::release::Uninstall;
/// use ironflow_core::operation::Operation;
///
/// let client = HelmClient::default();
/// let op = Uninstall::new(client, "my-release");
/// assert_eq!(op.kind(), "helm");
/// ```
pub struct Uninstall {
    client: HelmClient,
    name: String,
    keep_history: bool,
    cascade: Option<String>,
}

impl Uninstall {
    /// Create a new uninstall operation.
    pub fn new(client: HelmClient, name: impl Into<String>) -> Self {
        Self {
            client,
            name: name.into(),
            keep_history: false,
            cascade: None,
        }
    }

    /// Keep the release history.
    pub fn keep_history(mut self, keep: bool) -> Self {
        self.keep_history = keep;
        self
    }

    /// Set the cascade strategy (`background`, `orphan`, `foreground`).
    pub fn cascade(mut self, cascade: impl Into<String>) -> Self {
        self.cascade = Some(cascade.into());
        self
    }

    /// Execute and return the raw text output.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Shell`] if the uninstall fails.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<TextOutput, OperationError> {
        let mut extra = Vec::new();
        if self.keep_history {
            extra.push("--keep-history".to_string());
        }
        if let Some(ref c) = self.cascade {
            extra.push("--cascade".to_string());
            extra.push(c.clone());
        }
        let stdout = run_helm_with_extra(&self.client, &["uninstall", &self.name], &extra).await?;
        Ok(TextOutput {
            output: stdout.trim().to_string(),
        })
    }
}

#[async_trait]
impl Operation for Uninstall {
    fn kind(&self) -> &str {
        "helm"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "command": "uninstall", "name": self.name }))
    }
}

impl TypedOperation for Uninstall {
    type Output = TextOutput;
}

/// Rollback a release to a previous revision.
///
/// Wraps `helm rollback <name> [revision]`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_helm::HelmClient;
/// use ironflow_ops_helm::release::Rollback;
/// use ironflow_core::operation::Operation;
///
/// let client = HelmClient::default();
/// let op = Rollback::new(client, "my-release", 2);
/// assert_eq!(op.kind(), "helm");
/// ```
pub struct Rollback {
    client: HelmClient,
    name: String,
    revision: u32,
    wait: bool,
    timeout: Option<String>,
}

impl Rollback {
    /// Create a new rollback operation.
    pub fn new(client: HelmClient, name: impl Into<String>, revision: u32) -> Self {
        Self {
            client,
            name: name.into(),
            revision,
            wait: false,
            timeout: None,
        }
    }

    /// Wait for resources to be ready.
    pub fn wait(mut self, wait: bool) -> Self {
        self.wait = wait;
        self
    }

    /// Set the timeout for `--wait`.
    pub fn timeout(mut self, timeout: impl Into<String>) -> Self {
        self.timeout = Some(timeout.into());
        self
    }

    /// Execute and return the raw text output.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Shell`] if the rollback fails.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<TextOutput, OperationError> {
        let rev = self.revision.to_string();
        let mut extra = Vec::new();
        if self.wait {
            extra.push("--wait".to_string());
        }
        if let Some(ref t) = self.timeout {
            extra.push("--timeout".to_string());
            extra.push(t.clone());
        }
        let stdout =
            run_helm_with_extra(&self.client, &["rollback", &self.name, &rev], &extra).await?;
        Ok(TextOutput {
            output: stdout.trim().to_string(),
        })
    }
}

#[async_trait]
impl Operation for Rollback {
    fn kind(&self) -> &str {
        "helm"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "command": "rollback",
            "name": self.name,
            "revision": self.revision,
        }))
    }
}

impl TypedOperation for Rollback {
    type Output = TextOutput;
}

#[cfg(test)]
mod tests {
    use ironflow_core::operation::Operation;

    use super::*;

    #[test]
    fn uninstall_kind_and_input() {
        let op = Uninstall::new(HelmClient::default(), "rel");
        assert_eq!(op.kind(), "helm");
        let input = op.input().unwrap();
        assert_eq!(input["command"], "uninstall");
        assert_eq!(input["name"], "rel");
    }

    #[test]
    fn rollback_kind_and_input() {
        let op = Rollback::new(HelmClient::default(), "rel", 3);
        assert_eq!(op.kind(), "helm");
        let input = op.input().unwrap();
        assert_eq!(input["revision"], 3);
        assert_eq!(input["command"], "rollback");
    }
}
