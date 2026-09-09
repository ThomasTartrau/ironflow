//! Utility operations: version, env info, signature verification.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::client::HelmClient;
use crate::helpers::{run_helm, to_value};

/// Output of the `helm version` command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionOutput {
    /// The raw version string returned by Helm.
    pub version: String,
}

/// Run `helm version --short` to get the Helm version.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_helm::HelmClient;
/// use ironflow_ops_helm::util::Version;
/// use ironflow_core::operation::Operation;
///
/// let client = HelmClient::default();
/// let op = Version::new(client);
/// assert_eq!(op.kind(), "helm");
/// ```
pub struct Version {
    client: HelmClient,
}

impl Version {
    /// Create a new version operation.
    pub fn new(client: HelmClient) -> Self {
        Self { client }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Shell`] if the Helm binary exits with a
    /// non-zero status, or [`OperationError::External`] if the binary cannot
    /// be spawned.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<VersionOutput, OperationError> {
        let stdout = run_helm(&self.client, &["version", "--short"]).await?;
        Ok(VersionOutput {
            version: stdout.trim().to_string(),
        })
    }
}

#[async_trait]
impl Operation for Version {
    fn kind(&self) -> &str {
        "helm"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        None
    }
}

impl TypedOperation for Version {
    type Output = VersionOutput;
}

/// Output of the `helm env` command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvInfoOutput {
    /// Raw environment info as key-value pairs.
    pub entries: Vec<EnvEntry>,
}

/// A single Helm environment entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvEntry {
    /// The environment variable name.
    pub key: String,
    /// The environment variable value.
    pub value: String,
}

/// Run `helm env` to get Helm environment information.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_helm::HelmClient;
/// use ironflow_ops_helm::util::EnvInfo;
/// use ironflow_core::operation::Operation;
///
/// let client = HelmClient::default();
/// let op = EnvInfo::new(client);
/// assert_eq!(op.kind(), "helm");
/// ```
pub struct EnvInfo {
    client: HelmClient,
}

impl EnvInfo {
    /// Create a new env-info operation.
    pub fn new(client: HelmClient) -> Self {
        Self { client }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Shell`] if the Helm binary exits with a
    /// non-zero status, or [`OperationError::External`] if the binary cannot
    /// be spawned.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<EnvInfoOutput, OperationError> {
        let stdout = run_helm(&self.client, &["env"]).await?;
        let entries = stdout
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                let eq = line.find('=')?;
                let key = line[..eq].to_string();
                let raw_value = &line[eq + 1..];
                let value = raw_value.trim_matches('"').to_string();
                Some(EnvEntry { key, value })
            })
            .collect();
        Ok(EnvInfoOutput { entries })
    }
}

#[async_trait]
impl Operation for EnvInfo {
    fn kind(&self) -> &str {
        "helm"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        None
    }
}

impl TypedOperation for EnvInfo {
    type Output = EnvInfoOutput;
}

/// Run `helm verify` to verify a GPG-signed chart.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_helm::HelmClient;
/// use ironflow_ops_helm::util::Verify;
/// use ironflow_core::operation::Operation;
///
/// let client = HelmClient::default();
/// let op = Verify::new(client, "/path/to/chart.tgz");
/// assert_eq!(op.kind(), "helm");
/// ```
pub struct Verify {
    client: HelmClient,
    path: String,
    keyring: Option<String>,
}

impl Verify {
    /// Create a new verify operation.
    pub fn new(client: HelmClient, path: impl Into<String>) -> Self {
        Self {
            client,
            path: path.into(),
            keyring: None,
        }
    }

    /// Set a custom keyring path.
    pub fn keyring(mut self, keyring: impl Into<String>) -> Self {
        self.keyring = Some(keyring.into());
        self
    }

    /// Execute and return the verification output.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Shell`] if verification fails or the Helm
    /// binary exits with a non-zero status.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<TextOutput, OperationError> {
        let mut args = vec!["verify", &self.path];
        let keyring_val;
        if let Some(ref k) = self.keyring {
            keyring_val = k.clone();
            args.push("--keyring");
            args.push(&keyring_val);
        }
        let stdout = run_helm(&self.client, &args).await?;
        Ok(TextOutput {
            output: stdout.trim().to_string(),
        })
    }
}

#[async_trait]
impl Operation for Verify {
    fn kind(&self) -> &str {
        "helm"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "path": self.path }))
    }
}

impl TypedOperation for Verify {
    type Output = TextOutput;
}

/// Generic text output for commands that don't support JSON output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextOutput {
    /// The raw text output.
    pub output: String,
}
