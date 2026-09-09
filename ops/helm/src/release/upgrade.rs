//! `helm upgrade` operation.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde_json::Value;

use crate::client::HelmClient;
use crate::helpers::{run_helm_with_extra, to_value};
use crate::util::TextOutput;

/// Upgrade an existing release or install if it does not exist.
///
/// Wraps `helm upgrade <name> <chart>` with optional flags.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_helm::HelmClient;
/// use ironflow_ops_helm::release::Upgrade;
/// use ironflow_core::operation::Operation;
///
/// let client = HelmClient::default();
/// let op = Upgrade::new(client, "my-release", "bitnami/nginx")
///     .install(true)
///     .atomic(true);
/// assert_eq!(op.kind(), "helm");
/// ```
pub struct Upgrade {
    client: HelmClient,
    name: String,
    chart: String,
    values_files: Vec<String>,
    set_values: Vec<String>,
    install: bool,
    force: bool,
    reset_values: bool,
    reuse_values: bool,
    atomic: bool,
    wait: bool,
    timeout: Option<String>,
}

impl Upgrade {
    /// Create a new upgrade operation.
    pub fn new(client: HelmClient, name: impl Into<String>, chart: impl Into<String>) -> Self {
        Self {
            client,
            name: name.into(),
            chart: chart.into(),
            values_files: Vec::new(),
            set_values: Vec::new(),
            install: false,
            force: false,
            reset_values: false,
            reuse_values: false,
            atomic: false,
            wait: false,
            timeout: None,
        }
    }

    /// Add a values file.
    pub fn values(mut self, path: impl Into<String>) -> Self {
        self.values_files.push(path.into());
        self
    }

    /// Add a `--set` value override.
    pub fn set(mut self, key_value: impl Into<String>) -> Self {
        self.set_values.push(key_value.into());
        self
    }

    /// Install if the release does not exist.
    pub fn install(mut self, install: bool) -> Self {
        self.install = install;
        self
    }

    /// Force resource updates.
    pub fn force(mut self, force: bool) -> Self {
        self.force = force;
        self
    }

    /// Reset values to chart defaults.
    pub fn reset_values(mut self, reset: bool) -> Self {
        self.reset_values = reset;
        self
    }

    /// Reuse the last release's values.
    pub fn reuse_values(mut self, reuse: bool) -> Self {
        self.reuse_values = reuse;
        self
    }

    /// Roll back on failure.
    pub fn atomic(mut self, atomic: bool) -> Self {
        self.atomic = atomic;
        self
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
    /// Returns [`OperationError::Shell`] if the upgrade fails.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<TextOutput, OperationError> {
        let mut extra = Vec::new();
        for f in &self.values_files {
            extra.push("--values".to_string());
            extra.push(f.clone());
        }
        for s in &self.set_values {
            extra.push("--set".to_string());
            extra.push(s.clone());
        }
        if self.install {
            extra.push("--install".to_string());
        }
        if self.force {
            extra.push("--force".to_string());
        }
        if self.reset_values {
            extra.push("--reset-values".to_string());
        }
        if self.reuse_values {
            extra.push("--reuse-values".to_string());
        }
        if self.atomic {
            extra.push("--atomic".to_string());
        }
        if self.wait {
            extra.push("--wait".to_string());
        }
        if let Some(ref t) = self.timeout {
            extra.push("--timeout".to_string());
            extra.push(t.clone());
        }
        let stdout =
            run_helm_with_extra(&self.client, &["upgrade", &self.name, &self.chart], &extra)
                .await?;
        Ok(TextOutput {
            output: stdout.trim().to_string(),
        })
    }
}

#[async_trait]
impl Operation for Upgrade {
    fn kind(&self) -> &str {
        "helm"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "command": "upgrade",
            "name": self.name,
            "chart": self.chart,
        }))
    }
}

impl TypedOperation for Upgrade {
    type Output = TextOutput;
}

#[cfg(test)]
mod tests {
    use ironflow_core::operation::Operation;

    use super::*;

    #[test]
    fn kind_and_input() {
        let op = Upgrade::new(HelmClient::default(), "my-rel", "my-chart");
        assert_eq!(op.kind(), "helm");
        let input = op.input().unwrap();
        assert_eq!(input["command"], "upgrade");
        assert_eq!(input["name"], "my-rel");
        assert_eq!(input["chart"], "my-chart");
    }
}
