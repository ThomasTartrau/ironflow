//! Chart rendering: template, lint, package.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde_json::Value;

use crate::client::HelmClient;
use crate::helpers::{run_helm_with_extra, to_value};
use crate::util::TextOutput;

/// Render chart templates locally without deploying.
///
/// Wraps `helm template [name] <chart>`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_helm::HelmClient;
/// use ironflow_ops_helm::chart::Template;
/// use ironflow_core::operation::Operation;
///
/// let client = HelmClient::default();
/// let op = Template::new(client, "bitnami/nginx");
/// assert_eq!(op.kind(), "helm");
/// ```
pub struct Template {
    client: HelmClient,
    chart: String,
    name: Option<String>,
    values_files: Vec<String>,
    set_values: Vec<String>,
}

impl Template {
    /// Create a new template operation.
    pub fn new(client: HelmClient, chart: impl Into<String>) -> Self {
        Self {
            client,
            chart: chart.into(),
            name: None,
            values_files: Vec::new(),
            set_values: Vec::new(),
        }
    }

    /// Set the release name for the template.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
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

    /// Execute and return the rendered manifests.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Shell`] if the template rendering fails.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<TextOutput, OperationError> {
        let mut base_args = vec!["template"];
        let name_val;
        if let Some(ref n) = self.name {
            name_val = n.clone();
            base_args.push(&name_val);
        }
        base_args.push(&self.chart);
        let mut extra = Vec::new();
        for f in &self.values_files {
            extra.push("--values".to_string());
            extra.push(f.clone());
        }
        for s in &self.set_values {
            extra.push("--set".to_string());
            extra.push(s.clone());
        }
        let stdout = run_helm_with_extra(&self.client, &base_args, &extra).await?;
        Ok(TextOutput {
            output: stdout.trim().to_string(),
        })
    }
}

#[async_trait]
impl Operation for Template {
    fn kind(&self) -> &str {
        "helm"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "command": "template", "chart": self.chart }))
    }
}

impl TypedOperation for Template {
    type Output = TextOutput;
}

/// Lint a Helm chart.
///
/// Wraps `helm lint <path>`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_helm::HelmClient;
/// use ironflow_ops_helm::chart::Lint;
/// use ironflow_core::operation::Operation;
///
/// let client = HelmClient::default();
/// let op = Lint::new(client, "./my-chart");
/// assert_eq!(op.kind(), "helm");
/// ```
pub struct Lint {
    client: HelmClient,
    path: String,
    strict: bool,
}

impl Lint {
    /// Create a new lint operation.
    pub fn new(client: HelmClient, path: impl Into<String>) -> Self {
        Self {
            client,
            path: path.into(),
            strict: false,
        }
    }

    /// Enable strict mode.
    pub fn strict(mut self, strict: bool) -> Self {
        self.strict = strict;
        self
    }

    /// Execute and return the lint output.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Shell`] if the lint fails.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<TextOutput, OperationError> {
        let mut extra = Vec::new();
        if self.strict {
            extra.push("--strict".to_string());
        }
        let stdout = run_helm_with_extra(&self.client, &["lint", &self.path], &extra).await?;
        Ok(TextOutput {
            output: stdout.trim().to_string(),
        })
    }
}

#[async_trait]
impl Operation for Lint {
    fn kind(&self) -> &str {
        "helm"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "command": "lint", "path": self.path }))
    }
}

impl TypedOperation for Lint {
    type Output = TextOutput;
}

/// Package a chart directory into a chart archive.
///
/// Wraps `helm package <path>`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_helm::HelmClient;
/// use ironflow_ops_helm::chart::Package;
/// use ironflow_core::operation::Operation;
///
/// let client = HelmClient::default();
/// let op = Package::new(client, "./my-chart");
/// assert_eq!(op.kind(), "helm");
/// ```
pub struct Package {
    client: HelmClient,
    path: String,
    destination: Option<String>,
    version: Option<String>,
    app_version: Option<String>,
}

impl Package {
    /// Create a new package operation.
    pub fn new(client: HelmClient, path: impl Into<String>) -> Self {
        Self {
            client,
            path: path.into(),
            destination: None,
            version: None,
            app_version: None,
        }
    }

    /// Set the output directory.
    pub fn destination(mut self, destination: impl Into<String>) -> Self {
        self.destination = Some(destination.into());
        self
    }

    /// Override the chart version.
    pub fn version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    /// Override the app version.
    pub fn app_version(mut self, app_version: impl Into<String>) -> Self {
        self.app_version = Some(app_version.into());
        self
    }

    /// Execute and return the package output.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Shell`] if the packaging fails.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<TextOutput, OperationError> {
        let mut extra = Vec::new();
        if let Some(ref d) = self.destination {
            extra.push("--destination".to_string());
            extra.push(d.clone());
        }
        if let Some(ref v) = self.version {
            extra.push("--version".to_string());
            extra.push(v.clone());
        }
        if let Some(ref a) = self.app_version {
            extra.push("--app-version".to_string());
            extra.push(a.clone());
        }
        let stdout = run_helm_with_extra(&self.client, &["package", &self.path], &extra).await?;
        Ok(TextOutput {
            output: stdout.trim().to_string(),
        })
    }
}

#[async_trait]
impl Operation for Package {
    fn kind(&self) -> &str {
        "helm"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "command": "package", "path": self.path }))
    }
}

impl TypedOperation for Package {
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
    fn template_kind_and_input() {
        let op = Template::new(client(), "bitnami/nginx");
        assert_eq!(op.kind(), "helm");
        let input = op.input().unwrap();
        assert_eq!(input["command"], "template");
    }

    #[test]
    fn lint_kind_and_input() {
        let op = Lint::new(client(), "./chart");
        assert_eq!(op.kind(), "helm");
        let input = op.input().unwrap();
        assert_eq!(input["command"], "lint");
    }

    #[test]
    fn package_kind_and_input() {
        let op = Package::new(client(), "./chart");
        assert_eq!(op.kind(), "helm");
        let input = op.input().unwrap();
        assert_eq!(input["command"], "package");
    }
}
