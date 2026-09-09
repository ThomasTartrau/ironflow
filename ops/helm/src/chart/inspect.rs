//! Chart inspection: show, pull, push.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde_json::Value;

use crate::client::HelmClient;
use crate::helpers::{run_helm, run_helm_with_extra, to_value};
use crate::util::TextOutput;

/// Show subcommand variants.
#[derive(Debug, Clone, Copy)]
pub enum ShowSubcommand {
    /// Show the chart definition.
    Chart,
    /// Show the chart README.
    Readme,
    /// Show the chart default values.
    Values,
    /// Show the chart CRDs.
    Crds,
    /// Show all chart info.
    All,
}

impl ShowSubcommand {
    fn as_str(&self) -> &str {
        match self {
            Self::Chart => "chart",
            Self::Readme => "readme",
            Self::Values => "values",
            Self::Crds => "crds",
            Self::All => "all",
        }
    }
}

/// Show chart information.
///
/// Wraps `helm show <subcommand> <chart>`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_helm::HelmClient;
/// use ironflow_ops_helm::chart::{Show, ShowSubcommand};
/// use ironflow_core::operation::Operation;
///
/// let client = HelmClient::default();
/// let op = Show::new(client, ShowSubcommand::Values, "bitnami/nginx");
/// assert_eq!(op.kind(), "helm");
/// ```
pub struct Show {
    client: HelmClient,
    subcommand: ShowSubcommand,
    chart: String,
}

impl Show {
    /// Create a new show operation.
    pub fn new(client: HelmClient, subcommand: ShowSubcommand, chart: impl Into<String>) -> Self {
        Self {
            client,
            subcommand,
            chart: chart.into(),
        }
    }

    /// Execute and return the show output.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Shell`] if the command fails.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<TextOutput, OperationError> {
        let sub = self.subcommand.as_str();
        let stdout = run_helm(&self.client, &["show", sub, &self.chart]).await?;
        Ok(TextOutput {
            output: stdout.trim().to_string(),
        })
    }
}

#[async_trait]
impl Operation for Show {
    fn kind(&self) -> &str {
        "helm"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "command": "show",
            "subcommand": self.subcommand.as_str(),
            "chart": self.chart,
        }))
    }
}

impl TypedOperation for Show {
    type Output = TextOutput;
}

/// Pull a chart from a repository.
///
/// Wraps `helm pull <chart>`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_helm::HelmClient;
/// use ironflow_ops_helm::chart::Pull;
/// use ironflow_core::operation::Operation;
///
/// let client = HelmClient::default();
/// let op = Pull::new(client, "bitnami/nginx");
/// assert_eq!(op.kind(), "helm");
/// ```
pub struct Pull {
    client: HelmClient,
    chart: String,
    version: Option<String>,
    destination: Option<String>,
    untar: bool,
}

impl Pull {
    /// Create a new pull operation.
    pub fn new(client: HelmClient, chart: impl Into<String>) -> Self {
        Self {
            client,
            chart: chart.into(),
            version: None,
            destination: None,
            untar: false,
        }
    }

    /// Pull a specific version.
    pub fn version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    /// Set the output directory.
    pub fn destination(mut self, destination: impl Into<String>) -> Self {
        self.destination = Some(destination.into());
        self
    }

    /// Untar the chart after download.
    pub fn untar(mut self, untar: bool) -> Self {
        self.untar = untar;
        self
    }

    /// Execute and return the pull output.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Shell`] if the pull fails.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<TextOutput, OperationError> {
        let mut extra = Vec::new();
        if let Some(ref v) = self.version {
            extra.push("--version".to_string());
            extra.push(v.clone());
        }
        if let Some(ref d) = self.destination {
            extra.push("--destination".to_string());
            extra.push(d.clone());
        }
        if self.untar {
            extra.push("--untar".to_string());
        }
        let stdout = run_helm_with_extra(&self.client, &["pull", &self.chart], &extra).await?;
        Ok(TextOutput {
            output: stdout.trim().to_string(),
        })
    }
}

#[async_trait]
impl Operation for Pull {
    fn kind(&self) -> &str {
        "helm"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "command": "pull", "chart": self.chart }))
    }
}

impl TypedOperation for Pull {
    type Output = TextOutput;
}

/// Push a chart to an OCI registry.
///
/// Wraps `helm push <chart> <remote>`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_helm::HelmClient;
/// use ironflow_ops_helm::chart::Push;
/// use ironflow_core::operation::Operation;
///
/// let client = HelmClient::default();
/// let op = Push::new(client, "mychart-0.1.0.tgz", "oci://registry.example.com/charts");
/// assert_eq!(op.kind(), "helm");
/// ```
pub struct Push {
    client: HelmClient,
    chart: String,
    remote: String,
}

impl Push {
    /// Create a new push operation.
    pub fn new(client: HelmClient, chart: impl Into<String>, remote: impl Into<String>) -> Self {
        Self {
            client,
            chart: chart.into(),
            remote: remote.into(),
        }
    }

    /// Execute and return the push output.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Shell`] if the push fails.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<TextOutput, OperationError> {
        let stdout = run_helm(&self.client, &["push", &self.chart, &self.remote]).await?;
        Ok(TextOutput {
            output: stdout.trim().to_string(),
        })
    }
}

#[async_trait]
impl Operation for Push {
    fn kind(&self) -> &str {
        "helm"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "command": "push",
            "chart": self.chart,
            "remote": self.remote,
        }))
    }
}

impl TypedOperation for Push {
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
    fn show_kind_and_input() {
        let op = Show::new(client(), ShowSubcommand::Values, "bitnami/nginx");
        assert_eq!(op.kind(), "helm");
        let input = op.input().unwrap();
        assert_eq!(input["subcommand"], "values");
    }

    #[test]
    fn pull_kind() {
        let op = Pull::new(client(), "bitnami/nginx");
        assert_eq!(op.kind(), "helm");
    }

    #[test]
    fn push_kind_and_input() {
        let op = Push::new(client(), "chart.tgz", "oci://registry.example.com");
        assert_eq!(op.kind(), "helm");
        let input = op.input().unwrap();
        assert_eq!(input["command"], "push");
    }
}
