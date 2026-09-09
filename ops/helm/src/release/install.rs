//! `helm install` operation.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde_json::Value;

use crate::client::HelmClient;
use crate::helpers::{run_helm_with_extra, to_value};
use crate::util::TextOutput;

/// Install a Helm chart as a new release.
///
/// Wraps `helm install <name> <chart>` with optional flags.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_helm::HelmClient;
/// use ironflow_ops_helm::release::Install;
/// use ironflow_core::operation::Operation;
///
/// let client = HelmClient::default();
/// let op = Install::new(client, "my-release", "bitnami/nginx")
///     .wait(true)
///     .timeout("5m0s")
///     .create_namespace(true);
/// assert_eq!(op.kind(), "helm");
/// ```
pub struct Install {
    client: HelmClient,
    name: String,
    chart: String,
    values_files: Vec<String>,
    set_values: Vec<String>,
    wait: bool,
    timeout: Option<String>,
    create_namespace: bool,
}

impl Install {
    /// Create a new install operation.
    pub fn new(client: HelmClient, name: impl Into<String>, chart: impl Into<String>) -> Self {
        Self {
            client,
            name: name.into(),
            chart: chart.into(),
            values_files: Vec::new(),
            set_values: Vec::new(),
            wait: false,
            timeout: None,
            create_namespace: false,
        }
    }

    /// Add a values file (`-f` / `--values`).
    pub fn values(mut self, path: impl Into<String>) -> Self {
        self.values_files.push(path.into());
        self
    }

    /// Add a `--set` value override.
    pub fn set(mut self, key_value: impl Into<String>) -> Self {
        self.set_values.push(key_value.into());
        self
    }

    /// Wait for resources to be ready before marking the release as successful.
    pub fn wait(mut self, wait: bool) -> Self {
        self.wait = wait;
        self
    }

    /// Set the timeout for `--wait`.
    pub fn timeout(mut self, timeout: impl Into<String>) -> Self {
        self.timeout = Some(timeout.into());
        self
    }

    /// Create the namespace if it does not exist.
    pub fn create_namespace(mut self, create: bool) -> Self {
        self.create_namespace = create;
        self
    }

    /// Execute and return the raw text output.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Shell`] if the install fails.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<TextOutput, OperationError> {
        let stdout = run_helm_with_extra(
            &self.client,
            &["install", &self.name, &self.chart],
            &self.extra_args(),
        )
        .await?;
        Ok(TextOutput {
            output: stdout.trim().to_string(),
        })
    }

    pub(crate) fn extra_args(&self) -> Vec<String> {
        let mut extra = Vec::new();
        for f in &self.values_files {
            extra.push("--values".to_string());
            extra.push(f.clone());
        }
        for s in &self.set_values {
            extra.push("--set".to_string());
            extra.push(s.clone());
        }
        if self.wait {
            extra.push("--wait".to_string());
        }
        if let Some(ref t) = self.timeout {
            extra.push("--timeout".to_string());
            extra.push(t.clone());
        }
        if self.create_namespace {
            extra.push("--create-namespace".to_string());
        }
        extra
    }
}

#[async_trait]
impl Operation for Install {
    fn kind(&self) -> &str {
        "helm"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "command": "install",
            "name": self.name,
            "chart": self.chart,
        }))
    }
}

impl TypedOperation for Install {
    type Output = TextOutput;
}

#[cfg(test)]
mod tests {
    use ironflow_core::operation::Operation;

    use super::*;

    #[test]
    fn kind_and_input() {
        let op = Install::new(HelmClient::default(), "my-release", "bitnami/nginx");
        assert_eq!(op.kind(), "helm");
        let input = op.input().unwrap();
        assert_eq!(input["name"], "my-release");
        assert_eq!(input["chart"], "bitnami/nginx");
        assert_eq!(input["command"], "install");
    }

    #[test]
    fn extra_args_with_all_flags() {
        let op = Install::new(HelmClient::default(), "rel", "chart")
            .values("values.yaml")
            .set("key=val")
            .wait(true)
            .timeout("5m")
            .create_namespace(true);
        let args = op.extra_args();
        assert!(args.contains(&"--values".to_string()));
        assert!(args.contains(&"values.yaml".to_string()));
        assert!(args.contains(&"--set".to_string()));
        assert!(args.contains(&"key=val".to_string()));
        assert!(args.contains(&"--wait".to_string()));
        assert!(args.contains(&"--timeout".to_string()));
        assert!(args.contains(&"5m".to_string()));
        assert!(args.contains(&"--create-namespace".to_string()));
    }

    #[test]
    fn extra_args_empty_by_default() {
        let op = Install::new(HelmClient::default(), "rel", "chart");
        assert!(op.extra_args().is_empty());
    }
}
