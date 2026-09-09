//! Remote operations.

use std::path::PathBuf;

use async_trait::async_trait;
use git2::Repository;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::helpers::{blocking, to_value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteCreateOutput {
    pub name: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteDeleteOutput {
    pub name: String,
    pub deleted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteRenameOutput {
    pub old_name: String,
    pub new_name: String,
    pub problems: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteSetUrlOutput {
    pub name: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteEntry {
    pub name: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteListOutput {
    pub remotes: Vec<RemoteEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteLookupOutput {
    pub name: String,
    pub url: String,
    pub pushurl: Option<String>,
}

/// Create a new remote.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::remote::RemoteCreate;
/// use ironflow_core::operation::Operation;
///
/// let op = RemoteCreate::new("/path/to/repo", "origin", "https://example.com/repo.git");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct RemoteCreate {
    repo_path: PathBuf,
    name: String,
    url: String,
}

impl RemoteCreate {
    /// Create a new remote-create operation.
    pub fn new(
        repo_path: impl Into<PathBuf>,
        name: impl Into<String>,
        url: impl Into<String>,
    ) -> Self {
        Self {
            repo_path: repo_path.into(),
            name: name.into(),
            url: url.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<RemoteCreateOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let name = self.name.clone();
        let url = self.url.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            repo.remote(&name, &url)?;
            Ok(RemoteCreateOutput { name, url })
        })
        .await
    }
}

#[async_trait]
impl Operation for RemoteCreate {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "repo_path": self.repo_path, "name": self.name, "url": self.url }))
    }
}

impl TypedOperation for RemoteCreate {
    type Output = RemoteCreateOutput;
}

/// Delete a remote.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::remote::RemoteDelete;
/// use ironflow_core::operation::Operation;
///
/// let op = RemoteDelete::new("/path/to/repo", "origin");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct RemoteDelete {
    repo_path: PathBuf,
    name: String,
}

impl RemoteDelete {
    /// Create a new remote-delete operation.
    pub fn new(repo_path: impl Into<PathBuf>, name: impl Into<String>) -> Self {
        Self {
            repo_path: repo_path.into(),
            name: name.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<RemoteDeleteOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let name = self.name.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            repo.remote_delete(&name)?;
            Ok(RemoteDeleteOutput {
                name,
                deleted: true,
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for RemoteDelete {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "repo_path": self.repo_path, "name": self.name }))
    }
}

impl TypedOperation for RemoteDelete {
    type Output = RemoteDeleteOutput;
}

/// Rename a remote.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::remote::RemoteRename;
/// use ironflow_core::operation::Operation;
///
/// let op = RemoteRename::new("/path/to/repo", "origin", "upstream");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct RemoteRename {
    repo_path: PathBuf,
    old_name: String,
    new_name: String,
}

impl RemoteRename {
    /// Create a new remote-rename operation.
    pub fn new(
        repo_path: impl Into<PathBuf>,
        old_name: impl Into<String>,
        new_name: impl Into<String>,
    ) -> Self {
        Self {
            repo_path: repo_path.into(),
            old_name: old_name.into(),
            new_name: new_name.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<RemoteRenameOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let old = self.old_name.clone();
        let new = self.new_name.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let problems = repo.remote_rename(&old, &new)?;
            let issues: Vec<String> = problems
                .iter()
                .filter_map(|s| s.map(String::from))
                .collect();
            Ok(RemoteRenameOutput {
                old_name: old,
                new_name: new,
                problems: issues,
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for RemoteRename {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(
            serde_json::json!({ "repo_path": self.repo_path, "old_name": self.old_name, "new_name": self.new_name }),
        )
    }
}

impl TypedOperation for RemoteRename {
    type Output = RemoteRenameOutput;
}

/// Set the URL of a remote.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::remote::RemoteSetUrl;
/// use ironflow_core::operation::Operation;
///
/// let op = RemoteSetUrl::new("/path/to/repo", "origin", "https://new-url.com/repo.git");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct RemoteSetUrl {
    repo_path: PathBuf,
    name: String,
    url: String,
}

impl RemoteSetUrl {
    /// Create a new set-url operation.
    pub fn new(
        repo_path: impl Into<PathBuf>,
        name: impl Into<String>,
        url: impl Into<String>,
    ) -> Self {
        Self {
            repo_path: repo_path.into(),
            name: name.into(),
            url: url.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<RemoteSetUrlOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let name = self.name.clone();
        let url = self.url.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            repo.remote_set_url(&name, &url)?;
            Ok(RemoteSetUrlOutput { name, url })
        })
        .await
    }
}

#[async_trait]
impl Operation for RemoteSetUrl {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "repo_path": self.repo_path, "name": self.name, "url": self.url }))
    }
}

impl TypedOperation for RemoteSetUrl {
    type Output = RemoteSetUrlOutput;
}

/// List all remotes.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::remote::RemoteList;
/// use ironflow_core::operation::Operation;
///
/// let op = RemoteList::new("/path/to/repo");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct RemoteList {
    repo_path: PathBuf,
}

impl RemoteList {
    /// Create a new remote-list operation.
    pub fn new(repo_path: impl Into<PathBuf>) -> Self {
        Self {
            repo_path: repo_path.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<RemoteListOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let remotes = repo.remotes()?;
            let list: Vec<RemoteEntry> = remotes
                .iter()
                .filter_map(|name| {
                    let name = name?;
                    let remote = repo.find_remote(name).ok()?;
                    Some(RemoteEntry {
                        name: name.to_string(),
                        url: remote.url().unwrap_or("").to_string(),
                    })
                })
                .collect();
            Ok(RemoteListOutput { remotes: list })
        })
        .await
    }
}

#[async_trait]
impl Operation for RemoteList {
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

impl TypedOperation for RemoteList {
    type Output = RemoteListOutput;
}

/// Look up a remote by name.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::remote::RemoteLookup;
/// use ironflow_core::operation::Operation;
///
/// let op = RemoteLookup::new("/path/to/repo", "origin");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct RemoteLookup {
    repo_path: PathBuf,
    name: String,
}

impl RemoteLookup {
    /// Create a new remote-lookup operation.
    pub fn new(repo_path: impl Into<PathBuf>, name: impl Into<String>) -> Self {
        Self {
            repo_path: repo_path.into(),
            name: name.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<RemoteLookupOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let name = self.name.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let remote = repo.find_remote(&name)?;
            Ok(RemoteLookupOutput {
                name,
                url: remote.url().unwrap_or("").to_string(),
                pushurl: remote.pushurl().map(String::from),
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for RemoteLookup {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "repo_path": self.repo_path, "name": self.name }))
    }
}

impl TypedOperation for RemoteLookup {
    type Output = RemoteLookupOutput;
}

#[cfg(test)]
mod tests {
    use git2::Repository;
    use ironflow_core::operation::Operation;

    use super::*;
    use crate::test_helpers::ctx;

    #[tokio::test]
    async fn create_and_lookup() {
        let tmp = tempfile::tempdir().unwrap();
        Repository::init(tmp.path()).unwrap();
        RemoteCreate::new(tmp.path(), "origin", "https://example.com/r.git")
            .run(&ctx())
            .await
            .unwrap();
        let result = RemoteLookup::new(tmp.path(), "origin")
            .run(&ctx())
            .await
            .unwrap();
        assert_eq!(result.name, "origin");
        assert_eq!(result.url, "https://example.com/r.git");
        assert!(result.pushurl.is_none());
    }

    #[tokio::test]
    async fn list_remotes() {
        let tmp = tempfile::tempdir().unwrap();
        Repository::init(tmp.path()).unwrap();
        RemoteCreate::new(tmp.path(), "origin", "https://a.com")
            .run(&ctx())
            .await
            .unwrap();
        RemoteCreate::new(tmp.path(), "upstream", "https://b.com")
            .run(&ctx())
            .await
            .unwrap();
        let result = RemoteList::new(tmp.path()).run(&ctx()).await.unwrap();
        assert_eq!(result.remotes.len(), 2);
        let names: Vec<&str> = result.remotes.iter().map(|r| r.name.as_str()).collect();
        assert!(names.contains(&"origin"));
        assert!(names.contains(&"upstream"));
    }

    #[tokio::test]
    async fn rename_remote() {
        let tmp = tempfile::tempdir().unwrap();
        Repository::init(tmp.path()).unwrap();
        RemoteCreate::new(tmp.path(), "old", "https://a.com")
            .run(&ctx())
            .await
            .unwrap();
        let result = RemoteRename::new(tmp.path(), "old", "new")
            .run(&ctx())
            .await
            .unwrap();
        assert_eq!(result.old_name, "old");
        assert_eq!(result.new_name, "new");
        assert!(
            RemoteLookup::new(tmp.path(), "new")
                .run(&ctx())
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn set_url() {
        let tmp = tempfile::tempdir().unwrap();
        Repository::init(tmp.path()).unwrap();
        RemoteCreate::new(tmp.path(), "origin", "https://old.com")
            .run(&ctx())
            .await
            .unwrap();
        RemoteSetUrl::new(tmp.path(), "origin", "https://new.com")
            .run(&ctx())
            .await
            .unwrap();
        let result = RemoteLookup::new(tmp.path(), "origin")
            .run(&ctx())
            .await
            .unwrap();
        assert_eq!(result.url, "https://new.com");
    }

    #[tokio::test]
    async fn delete_remote() {
        let tmp = tempfile::tempdir().unwrap();
        Repository::init(tmp.path()).unwrap();
        RemoteCreate::new(tmp.path(), "origin", "https://a.com")
            .run(&ctx())
            .await
            .unwrap();
        let result = RemoteDelete::new(tmp.path(), "origin")
            .run(&ctx())
            .await
            .unwrap();
        assert!(result.deleted);
        assert!(
            RemoteLookup::new(tmp.path(), "origin")
                .run(&ctx())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn lookup_missing_remote_fails() {
        let tmp = tempfile::tempdir().unwrap();
        Repository::init(tmp.path()).unwrap();
        assert!(
            RemoteLookup::new(tmp.path(), "nope")
                .run(&ctx())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn execute_serializes_correctly() {
        let tmp = tempfile::tempdir().unwrap();
        Repository::init(tmp.path()).unwrap();
        let value = RemoteList::new(tmp.path()).execute(&ctx()).await.unwrap();
        assert!(value["remotes"].as_array().unwrap().is_empty());
    }
}
