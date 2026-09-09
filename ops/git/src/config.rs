//! Git config operations.

use std::path::PathBuf;

use async_trait::async_trait;
use git2::Repository;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::helpers::{blocking, to_value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigGetOutput {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigSetOutput {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigDeleteOutput {
    pub key: String,
    pub deleted: bool,
}

/// A single config entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigEntry {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigListOutput {
    pub entries: Vec<ConfigEntry>,
}

/// Get a config value.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::config::ConfigGet;
/// use ironflow_core::operation::Operation;
///
/// let op = ConfigGet::new("/path/to/repo", "user.name");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct ConfigGet {
    repo_path: PathBuf,
    key: String,
}

impl ConfigGet {
    /// Create a new config-get operation.
    pub fn new(repo_path: impl Into<PathBuf>, key: impl Into<String>) -> Self {
        Self {
            repo_path: repo_path.into(),
            key: key.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<ConfigGetOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let key = self.key.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let config = repo.config()?;
            let value = config.get_string(&key)?;
            Ok(ConfigGetOutput { key, value })
        })
        .await
    }
}

#[async_trait]
impl Operation for ConfigGet {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "repo_path": self.repo_path, "key": self.key }))
    }
}

impl TypedOperation for ConfigGet {
    type Output = ConfigGetOutput;
}

/// Set a config value.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::config::ConfigSet;
/// use ironflow_core::operation::Operation;
///
/// let op = ConfigSet::new("/path/to/repo", "user.name", "Alice");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct ConfigSet {
    repo_path: PathBuf,
    key: String,
    value: String,
}

impl ConfigSet {
    /// Create a new config-set operation.
    pub fn new(
        repo_path: impl Into<PathBuf>,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        Self {
            repo_path: repo_path.into(),
            key: key.into(),
            value: value.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<ConfigSetOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let key = self.key.clone();
        let value = self.value.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let mut config = repo.config()?;
            config.set_str(&key, &value)?;
            Ok(ConfigSetOutput { key, value })
        })
        .await
    }
}

#[async_trait]
impl Operation for ConfigSet {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(
            serde_json::json!({ "repo_path": self.repo_path, "key": self.key, "value": self.value }),
        )
    }
}

impl TypedOperation for ConfigSet {
    type Output = ConfigSetOutput;
}

/// Delete a config entry.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::config::ConfigDelete;
/// use ironflow_core::operation::Operation;
///
/// let op = ConfigDelete::new("/path/to/repo", "user.name");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct ConfigDelete {
    repo_path: PathBuf,
    key: String,
}

impl ConfigDelete {
    /// Create a new config-delete operation.
    pub fn new(repo_path: impl Into<PathBuf>, key: impl Into<String>) -> Self {
        Self {
            repo_path: repo_path.into(),
            key: key.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<ConfigDeleteOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let key = self.key.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let mut config = repo.config()?;
            config.remove(&key)?;
            Ok(ConfigDeleteOutput { key, deleted: true })
        })
        .await
    }
}

#[async_trait]
impl Operation for ConfigDelete {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "repo_path": self.repo_path, "key": self.key }))
    }
}

impl TypedOperation for ConfigDelete {
    type Output = ConfigDeleteOutput;
}

/// List all config entries.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::config::ConfigList;
/// use ironflow_core::operation::Operation;
///
/// let op = ConfigList::new("/path/to/repo");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct ConfigList {
    repo_path: PathBuf,
}

impl ConfigList {
    /// Create a new config-list operation.
    pub fn new(repo_path: impl Into<PathBuf>) -> Self {
        Self {
            repo_path: repo_path.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<ConfigListOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let mut config = repo.config()?;
            let snapshot = config.snapshot()?;
            let mut entries = Vec::new();
            let mut iter = snapshot.entries(None)?;
            while let Some(entry) = iter.next() {
                let entry = entry?;
                if let (Some(name), Some(value)) = (entry.name(), entry.value()) {
                    entries.push(ConfigEntry {
                        name: name.to_string(),
                        value: value.to_string(),
                    });
                }
            }
            Ok(ConfigListOutput { entries })
        })
        .await
    }
}

#[async_trait]
impl Operation for ConfigList {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "repo_path": self.repo_path }))
    }
}

impl TypedOperation for ConfigList {
    type Output = ConfigListOutput;
}

#[cfg(test)]
mod tests {
    use git2::Repository;
    use ironflow_core::operation::Operation;

    use super::*;
    use crate::test_helpers::ctx;

    #[tokio::test]
    async fn set_and_get() {
        let tmp = tempfile::tempdir().unwrap();
        Repository::init(tmp.path()).unwrap();
        ConfigSet::new(tmp.path(), "user.name", "Alice")
            .run(&ctx())
            .await
            .unwrap();
        let result = ConfigGet::new(tmp.path(), "user.name")
            .run(&ctx())
            .await
            .unwrap();
        assert_eq!(result.key, "user.name");
        assert_eq!(result.value, "Alice");
    }

    #[tokio::test]
    async fn get_missing_key_fails() {
        let tmp = tempfile::tempdir().unwrap();
        Repository::init(tmp.path()).unwrap();
        assert!(
            ConfigGet::new(tmp.path(), "no.such.key")
                .run(&ctx())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn delete_removes_key() {
        let tmp = tempfile::tempdir().unwrap();
        Repository::init(tmp.path()).unwrap();
        ConfigSet::new(tmp.path(), "test.deletekey", "val")
            .run(&ctx())
            .await
            .unwrap();
        let result = ConfigDelete::new(tmp.path(), "test.deletekey")
            .run(&ctx())
            .await
            .unwrap();
        assert!(result.deleted);
        assert!(
            ConfigGet::new(tmp.path(), "test.deletekey")
                .run(&ctx())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn list_includes_set_key() {
        let tmp = tempfile::tempdir().unwrap();
        Repository::init(tmp.path()).unwrap();
        ConfigSet::new(tmp.path(), "test.key", "val")
            .run(&ctx())
            .await
            .unwrap();
        let result = ConfigList::new(tmp.path()).run(&ctx()).await.unwrap();
        assert!(
            result
                .entries
                .iter()
                .any(|e| e.name == "test.key" && e.value == "val")
        );
    }

    #[tokio::test]
    async fn execute_serializes_correctly() {
        let tmp = tempfile::tempdir().unwrap();
        Repository::init(tmp.path()).unwrap();
        ConfigSet::new(tmp.path(), "user.name", "Bob")
            .run(&ctx())
            .await
            .unwrap();
        let value = ConfigGet::new(tmp.path(), "user.name")
            .execute(&ctx())
            .await
            .unwrap();
        assert_eq!(value["value"], "Bob");
    }
}
