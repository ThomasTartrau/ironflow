//! Repository management: add, remove, update, list, index.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::client::HelmClient;
use crate::helpers::{run_helm, run_helm_json, run_helm_with_extra, to_value};
use crate::util::TextOutput;

/// Add a chart repository.
///
/// Wraps `helm repo add <name> <url>`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_helm::HelmClient;
/// use ironflow_ops_helm::repo::RepoAdd;
/// use ironflow_core::operation::Operation;
///
/// let client = HelmClient::default();
/// let op = RepoAdd::new(client, "bitnami", "https://charts.bitnami.com/bitnami");
/// assert_eq!(op.kind(), "helm");
/// ```
pub struct RepoAdd {
    client: HelmClient,
    name: String,
    url: String,
    force_update: bool,
}

impl RepoAdd {
    /// Create a new repo-add operation.
    pub fn new(client: HelmClient, name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            client,
            name: name.into(),
            url: url.into(),
            force_update: false,
        }
    }

    /// Force update if the repo already exists.
    pub fn force_update(mut self, force: bool) -> Self {
        self.force_update = force;
        self
    }

    /// Execute and return the output.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Shell`] if the command fails.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<TextOutput, OperationError> {
        let mut extra = Vec::new();
        if self.force_update {
            extra.push("--force-update".to_string());
        }
        let stdout = run_helm_with_extra(
            &self.client,
            &["repo", "add", &self.name, &self.url],
            &extra,
        )
        .await?;
        Ok(TextOutput {
            output: stdout.trim().to_string(),
        })
    }
}

#[async_trait]
impl Operation for RepoAdd {
    fn kind(&self) -> &str {
        "helm"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "command": "repo add",
            "name": self.name,
            "url": self.url,
        }))
    }
}

impl TypedOperation for RepoAdd {
    type Output = TextOutput;
}

/// Remove a chart repository.
///
/// Wraps `helm repo remove <name>`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_helm::HelmClient;
/// use ironflow_ops_helm::repo::RepoRemove;
/// use ironflow_core::operation::Operation;
///
/// let client = HelmClient::default();
/// let op = RepoRemove::new(client, "bitnami");
/// assert_eq!(op.kind(), "helm");
/// ```
pub struct RepoRemove {
    client: HelmClient,
    name: String,
}

impl RepoRemove {
    /// Create a new repo-remove operation.
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
        let stdout = run_helm(&self.client, &["repo", "remove", &self.name]).await?;
        Ok(TextOutput {
            output: stdout.trim().to_string(),
        })
    }
}

#[async_trait]
impl Operation for RepoRemove {
    fn kind(&self) -> &str {
        "helm"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "command": "repo remove", "name": self.name }))
    }
}

impl TypedOperation for RepoRemove {
    type Output = TextOutput;
}

/// Update all chart repositories.
///
/// Wraps `helm repo update`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_helm::HelmClient;
/// use ironflow_ops_helm::repo::RepoUpdate;
/// use ironflow_core::operation::Operation;
///
/// let client = HelmClient::default();
/// let op = RepoUpdate::new(client);
/// assert_eq!(op.kind(), "helm");
/// ```
pub struct RepoUpdate {
    client: HelmClient,
}

impl RepoUpdate {
    /// Create a new repo-update operation.
    pub fn new(client: HelmClient) -> Self {
        Self { client }
    }

    /// Execute and return the output.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Shell`] if the command fails.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<TextOutput, OperationError> {
        let stdout = run_helm(&self.client, &["repo", "update"]).await?;
        Ok(TextOutput {
            output: stdout.trim().to_string(),
        })
    }
}

#[async_trait]
impl Operation for RepoUpdate {
    fn kind(&self) -> &str {
        "helm"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "command": "repo update" }))
    }
}

impl TypedOperation for RepoUpdate {
    type Output = TextOutput;
}

/// A single repository entry from `helm repo list`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoEntry {
    /// Repository name.
    pub name: Option<String>,
    /// Repository URL.
    pub url: Option<String>,
}

/// List chart repositories.
///
/// Wraps `helm repo list --output json`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_helm::HelmClient;
/// use ironflow_ops_helm::repo::RepoList;
/// use ironflow_core::operation::Operation;
///
/// let client = HelmClient::default();
/// let op = RepoList::new(client);
/// assert_eq!(op.kind(), "helm");
/// ```
pub struct RepoList {
    client: HelmClient,
}

impl RepoList {
    /// Create a new repo-list operation.
    pub fn new(client: HelmClient) -> Self {
        Self { client }
    }

    /// Execute and return the list of repositories.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Shell`] if the command fails.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<Vec<RepoEntry>, OperationError> {
        run_helm_json(&self.client, &["repo", "list"]).await
    }
}

#[async_trait]
impl Operation for RepoList {
    fn kind(&self) -> &str {
        "helm"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "command": "repo list" }))
    }
}

impl TypedOperation for RepoList {
    type Output = Vec<RepoEntry>;
}

/// Generate an index file for a chart repository directory.
///
/// Wraps `helm repo index <dir>`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_helm::HelmClient;
/// use ironflow_ops_helm::repo::RepoIndex;
/// use ironflow_core::operation::Operation;
///
/// let client = HelmClient::default();
/// let op = RepoIndex::new(client, "./repo-dir");
/// assert_eq!(op.kind(), "helm");
/// ```
pub struct RepoIndex {
    client: HelmClient,
    dir: String,
    url: Option<String>,
}

impl RepoIndex {
    /// Create a new repo-index operation.
    pub fn new(client: HelmClient, dir: impl Into<String>) -> Self {
        Self {
            client,
            dir: dir.into(),
            url: None,
        }
    }

    /// Set the base URL for the chart repository.
    pub fn url(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into());
        self
    }

    /// Execute and return the output.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Shell`] if the command fails.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<TextOutput, OperationError> {
        let mut extra = Vec::new();
        if let Some(ref u) = self.url {
            extra.push("--url".to_string());
            extra.push(u.clone());
        }
        let stdout =
            run_helm_with_extra(&self.client, &["repo", "index", &self.dir], &extra).await?;
        Ok(TextOutput {
            output: stdout.trim().to_string(),
        })
    }
}

#[async_trait]
impl Operation for RepoIndex {
    fn kind(&self) -> &str {
        "helm"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "command": "repo index", "dir": self.dir }))
    }
}

impl TypedOperation for RepoIndex {
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
    fn repo_add_kind_and_input() {
        let op = RepoAdd::new(client(), "bitnami", "https://charts.bitnami.com/bitnami");
        assert_eq!(op.kind(), "helm");
        let input = op.input().unwrap();
        assert_eq!(input["command"], "repo add");
        assert_eq!(input["name"], "bitnami");
    }

    #[test]
    fn repo_list_kind() {
        assert_eq!(RepoList::new(client()).kind(), "helm");
    }

    #[test]
    fn repo_remove_kind() {
        assert_eq!(RepoRemove::new(client(), "bitnami").kind(), "helm");
    }

    #[test]
    fn repo_update_kind() {
        assert_eq!(RepoUpdate::new(client()).kind(), "helm");
    }

    #[test]
    fn repo_index_kind() {
        assert_eq!(RepoIndex::new(client(), "./dir").kind(), "helm");
    }
}
