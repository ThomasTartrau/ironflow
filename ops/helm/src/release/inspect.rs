//! `helm get` and `helm test` operations.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde_json::Value;

use crate::client::HelmClient;
use crate::helpers::{run_helm, run_helm_with_extra, to_value};
use crate::util::TextOutput;

/// Get subcommand variants for `helm get`.
#[derive(Debug, Clone, Copy)]
pub enum GetSubcommand {
    /// Get release values.
    Values,
    /// Get release manifest.
    Manifest,
    /// Get release notes.
    Notes,
    /// Get release hooks.
    Hooks,
    /// Get all release information.
    All,
}

impl GetSubcommand {
    fn as_str(&self) -> &str {
        match self {
            Self::Values => "values",
            Self::Manifest => "manifest",
            Self::Notes => "notes",
            Self::Hooks => "hooks",
            Self::All => "all",
        }
    }
}

/// Get information about a release.
///
/// Wraps `helm get <subcommand> <name>`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_helm::HelmClient;
/// use ironflow_ops_helm::release::{Get, GetSubcommand};
/// use ironflow_core::operation::Operation;
///
/// let client = HelmClient::default();
/// let op = Get::new(client, GetSubcommand::Values, "my-release");
/// assert_eq!(op.kind(), "helm");
/// ```
pub struct Get {
    client: HelmClient,
    subcommand: GetSubcommand,
    name: String,
    revision: Option<u32>,
}

impl Get {
    /// Create a new get operation.
    pub fn new(client: HelmClient, subcommand: GetSubcommand, name: impl Into<String>) -> Self {
        Self {
            client,
            subcommand,
            name: name.into(),
            revision: None,
        }
    }

    /// Get from a specific revision.
    pub fn revision(mut self, revision: u32) -> Self {
        self.revision = Some(revision);
        self
    }

    /// Execute and return the text output.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Shell`] if the command fails.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<TextOutput, OperationError> {
        let sub = self.subcommand.as_str();
        let mut args = vec!["get", sub, &self.name];
        let rev;
        if let Some(r) = self.revision {
            rev = r.to_string();
            args.push("--revision");
            args.push(&rev);
        }
        let stdout = run_helm(&self.client, &args).await?;
        Ok(TextOutput {
            output: stdout.trim().to_string(),
        })
    }
}

#[async_trait]
impl Operation for Get {
    fn kind(&self) -> &str {
        "helm"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "command": "get",
            "subcommand": self.subcommand.as_str(),
            "name": self.name,
        }))
    }
}

impl TypedOperation for Get {
    type Output = TextOutput;
}

/// Run release test hooks.
///
/// Wraps `helm test <name>`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_helm::HelmClient;
/// use ironflow_ops_helm::release::Test;
/// use ironflow_core::operation::Operation;
///
/// let client = HelmClient::default();
/// let op = Test::new(client, "my-release");
/// assert_eq!(op.kind(), "helm");
/// ```
pub struct Test {
    client: HelmClient,
    name: String,
    timeout: Option<String>,
}

impl Test {
    /// Create a new test operation.
    pub fn new(client: HelmClient, name: impl Into<String>) -> Self {
        Self {
            client,
            name: name.into(),
            timeout: None,
        }
    }

    /// Set the test timeout.
    pub fn timeout(mut self, timeout: impl Into<String>) -> Self {
        self.timeout = Some(timeout.into());
        self
    }

    /// Execute and return the test output.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Shell`] if the tests fail.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<TextOutput, OperationError> {
        let mut extra = Vec::new();
        if let Some(ref t) = self.timeout {
            extra.push("--timeout".to_string());
            extra.push(t.clone());
        }
        let stdout = run_helm_with_extra(&self.client, &["test", &self.name], &extra).await?;
        Ok(TextOutput {
            output: stdout.trim().to_string(),
        })
    }
}

#[async_trait]
impl Operation for Test {
    fn kind(&self) -> &str {
        "helm"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "command": "test",
            "name": self.name,
        }))
    }
}

impl TypedOperation for Test {
    type Output = TextOutput;
}

#[cfg(test)]
mod tests {
    use ironflow_core::operation::Operation;

    use super::*;

    #[test]
    fn get_kind_and_input() {
        let op = Get::new(HelmClient::default(), GetSubcommand::Values, "rel");
        assert_eq!(op.kind(), "helm");
        let input = op.input().unwrap();
        assert_eq!(input["subcommand"], "values");
        assert_eq!(input["command"], "get");
    }

    #[test]
    fn test_kind_and_input() {
        let op = Test::new(HelmClient::default(), "rel");
        assert_eq!(op.kind(), "helm");
        let input = op.input().unwrap();
        assert_eq!(input["command"], "test");
    }
}
