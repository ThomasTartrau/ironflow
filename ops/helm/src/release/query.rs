//! Release query operations: list, status, history.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::client::HelmClient;
use crate::helpers::{run_helm_json, to_value};

/// A single release entry from `helm list`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseEntry {
    /// Release name.
    pub name: Option<String>,
    /// Release namespace.
    pub namespace: Option<String>,
    /// Current revision number.
    pub revision: Option<String>,
    /// Last update timestamp.
    pub updated: Option<String>,
    /// Release status.
    pub status: Option<String>,
    /// Chart name and version.
    pub chart: Option<String>,
    /// App version.
    pub app_version: Option<String>,
}

/// List Helm releases.
///
/// Wraps `helm list --output json`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_helm::HelmClient;
/// use ironflow_ops_helm::release::List;
/// use ironflow_core::operation::Operation;
///
/// let client = HelmClient::default();
/// let op = List::new(client);
/// assert_eq!(op.kind(), "helm");
/// ```
pub struct List {
    client: HelmClient,
    all_namespaces: bool,
    filter: Option<String>,
    deployed: bool,
    failed: bool,
    pending: bool,
}

impl List {
    /// Create a new list operation.
    pub fn new(client: HelmClient) -> Self {
        Self {
            client,
            all_namespaces: false,
            filter: None,
            deployed: false,
            failed: false,
            pending: false,
        }
    }

    /// List releases across all namespaces.
    pub fn all_namespaces(mut self, all: bool) -> Self {
        self.all_namespaces = all;
        self
    }

    /// Filter releases by name.
    pub fn filter(mut self, filter: impl Into<String>) -> Self {
        self.filter = Some(filter.into());
        self
    }

    /// Show only deployed releases.
    pub fn deployed(mut self, deployed: bool) -> Self {
        self.deployed = deployed;
        self
    }

    /// Show only failed releases.
    pub fn failed(mut self, failed: bool) -> Self {
        self.failed = failed;
        self
    }

    /// Show only pending releases.
    pub fn pending(mut self, pending: bool) -> Self {
        self.pending = pending;
        self
    }

    /// Execute and return the list of releases.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Shell`] if the command fails.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<Vec<ReleaseEntry>, OperationError> {
        let mut args = vec!["list"];
        if self.all_namespaces {
            args.push("--all-namespaces");
        }
        if self.deployed {
            args.push("--deployed");
        }
        if self.failed {
            args.push("--failed");
        }
        if self.pending {
            args.push("--pending");
        }
        let filter_val;
        if let Some(ref f) = self.filter {
            filter_val = f.clone();
            args.push("--filter");
            args.push(&filter_val);
        }
        run_helm_json(&self.client, &args).await
    }
}

#[async_trait]
impl Operation for List {
    fn kind(&self) -> &str {
        "helm"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "command": "list" }))
    }
}

impl TypedOperation for List {
    type Output = Vec<ReleaseEntry>;
}

/// Get the status of a Helm release.
///
/// Wraps `helm status <name> --output json`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_helm::HelmClient;
/// use ironflow_ops_helm::release::Status;
/// use ironflow_core::operation::Operation;
///
/// let client = HelmClient::default();
/// let op = Status::new(client, "my-release");
/// assert_eq!(op.kind(), "helm");
/// ```
pub struct Status {
    client: HelmClient,
    name: String,
    revision: Option<u32>,
}

impl Status {
    /// Create a new status operation.
    pub fn new(client: HelmClient, name: impl Into<String>) -> Self {
        Self {
            client,
            name: name.into(),
            revision: None,
        }
    }

    /// Get the status of a specific revision.
    pub fn revision(mut self, revision: u32) -> Self {
        self.revision = Some(revision);
        self
    }

    /// Execute and return the status as JSON.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Shell`] if the command fails.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let mut args = vec!["status", &self.name];
        let rev;
        if let Some(r) = self.revision {
            rev = r.to_string();
            args.push("--revision");
            args.push(&rev);
        }
        run_helm_json(&self.client, &args).await
    }
}

#[async_trait]
impl Operation for Status {
    fn kind(&self) -> &str {
        "helm"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        self.run(ctx).await
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "command": "status",
            "name": self.name,
        }))
    }
}

impl TypedOperation for Status {
    type Output = Value;
}

/// A single history entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// Revision number.
    pub revision: Option<u32>,
    /// Last update timestamp.
    pub updated: Option<String>,
    /// Release status.
    pub status: Option<String>,
    /// Chart name and version.
    pub chart: Option<String>,
    /// App version.
    pub app_version: Option<String>,
    /// Description/notes.
    pub description: Option<String>,
}

/// Get the release history.
///
/// Wraps `helm history <name> --output json`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_helm::HelmClient;
/// use ironflow_ops_helm::release::History;
/// use ironflow_core::operation::Operation;
///
/// let client = HelmClient::default();
/// let op = History::new(client, "my-release");
/// assert_eq!(op.kind(), "helm");
/// ```
pub struct History {
    client: HelmClient,
    name: String,
    max: Option<u32>,
}

impl History {
    /// Create a new history operation.
    pub fn new(client: HelmClient, name: impl Into<String>) -> Self {
        Self {
            client,
            name: name.into(),
            max: None,
        }
    }

    /// Limit the number of history entries.
    pub fn max(mut self, max: u32) -> Self {
        self.max = Some(max);
        self
    }

    /// Execute and return the history entries.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Shell`] if the command fails.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<Vec<HistoryEntry>, OperationError> {
        let mut args = vec!["history", &self.name];
        let max_str;
        if let Some(m) = self.max {
            max_str = m.to_string();
            args.push("--max");
            args.push(&max_str);
        }
        run_helm_json(&self.client, &args).await
    }
}

#[async_trait]
impl Operation for History {
    fn kind(&self) -> &str {
        "helm"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "command": "history",
            "name": self.name,
        }))
    }
}

impl TypedOperation for History {
    type Output = Vec<HistoryEntry>;
}

#[cfg(test)]
mod tests {
    use ironflow_core::operation::Operation;

    use super::*;

    #[test]
    fn list_kind() {
        let op = List::new(HelmClient::default());
        assert_eq!(op.kind(), "helm");
    }

    #[test]
    fn status_kind_and_input() {
        let op = Status::new(HelmClient::default(), "my-rel");
        assert_eq!(op.kind(), "helm");
        let input = op.input().unwrap();
        assert_eq!(input["name"], "my-rel");
        assert_eq!(input["command"], "status");
    }

    #[test]
    fn history_kind_and_input() {
        let op = History::new(HelmClient::default(), "rel");
        assert_eq!(op.kind(), "helm");
        let input = op.input().unwrap();
        assert_eq!(input["command"], "history");
    }
}
