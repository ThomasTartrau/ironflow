//! Status operations.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use git2::{Repository, Status, StatusOptions};
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::helpers::{blocking, to_value};

fn status_label(status: Status) -> String {
    let mut parts = Vec::new();
    if status.is_index_new() {
        parts.push("index_new");
    }
    if status.is_index_modified() {
        parts.push("index_modified");
    }
    if status.is_index_deleted() {
        parts.push("index_deleted");
    }
    if status.is_index_renamed() {
        parts.push("index_renamed");
    }
    if status.is_index_typechange() {
        parts.push("index_typechange");
    }
    if status.is_wt_new() {
        parts.push("wt_new");
    }
    if status.is_wt_modified() {
        parts.push("wt_modified");
    }
    if status.is_wt_deleted() {
        parts.push("wt_deleted");
    }
    if status.is_wt_typechange() {
        parts.push("wt_typechange");
    }
    if status.is_wt_renamed() {
        parts.push("wt_renamed");
    }
    if status.is_ignored() {
        parts.push("ignored");
    }
    if status.is_conflicted() {
        parts.push("conflicted");
    }
    if parts.is_empty() {
        parts.push("current");
    }
    parts.join(",")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusFileOutput {
    pub path: String,
    pub status: String,
}

/// A single status entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusEntry {
    pub path: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusListOutput {
    pub entries: Vec<StatusEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusShouldIgnoreOutput {
    pub path: String,
    pub ignored: bool,
}

/// Get the status of a single file.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::status::StatusFile;
/// use ironflow_core::operation::Operation;
///
/// let op = StatusFile::new("/path/to/repo", "file.txt");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct StatusFile {
    repo_path: PathBuf,
    path: String,
}

impl StatusFile {
    /// Create a new status-file operation.
    pub fn new(repo_path: impl Into<PathBuf>, path: impl Into<String>) -> Self {
        Self {
            repo_path: repo_path.into(),
            path: path.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<StatusFileOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let path = self.path.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let status = repo.status_file(Path::new(&path))?;
            Ok(StatusFileOutput {
                path,
                status: status_label(status),
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for StatusFile {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "repo_path": self.repo_path, "path": self.path }))
    }
}

impl TypedOperation for StatusFile {
    type Output = StatusFileOutput;
}

/// List the status of all files in the repository.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::status::StatusList;
/// use ironflow_core::operation::Operation;
///
/// let op = StatusList::new("/path/to/repo");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct StatusList {
    repo_path: PathBuf,
}

impl StatusList {
    /// Create a new status-list operation.
    pub fn new(repo_path: impl Into<PathBuf>) -> Self {
        Self {
            repo_path: repo_path.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<StatusListOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let mut opts = StatusOptions::new();
            opts.include_untracked(true);
            let statuses = repo.statuses(Some(&mut opts))?;
            let entries = statuses
                .iter()
                .map(|entry| StatusEntry {
                    path: entry.path().unwrap_or("").to_string(),
                    status: status_label(entry.status()),
                })
                .collect();
            Ok(StatusListOutput { entries })
        })
        .await
    }
}

#[async_trait]
impl Operation for StatusList {
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

impl TypedOperation for StatusList {
    type Output = StatusListOutput;
}

/// Check if a path should be ignored.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::status::StatusShouldIgnore;
/// use ironflow_core::operation::Operation;
///
/// let op = StatusShouldIgnore::new("/path/to/repo", "target/");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct StatusShouldIgnore {
    repo_path: PathBuf,
    path: String,
}

impl StatusShouldIgnore {
    /// Create a new should-ignore check operation.
    pub fn new(repo_path: impl Into<PathBuf>, path: impl Into<String>) -> Self {
        Self {
            repo_path: repo_path.into(),
            path: path.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(
        &self,
        _ctx: &OperationContext,
    ) -> Result<StatusShouldIgnoreOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let path = self.path.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let ignored = repo.status_should_ignore(Path::new(&path))?;
            Ok(StatusShouldIgnoreOutput { path, ignored })
        })
        .await
    }
}

#[async_trait]
impl Operation for StatusShouldIgnore {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "repo_path": self.repo_path, "path": self.path }))
    }
}

impl TypedOperation for StatusShouldIgnore {
    type Output = StatusShouldIgnoreOutput;
}

#[cfg(test)]
mod tests {
    use std::fs;

    use ironflow_core::operation::Operation;

    use super::*;
    use crate::test_helpers::{ctx, init_repo};

    #[tokio::test]
    async fn status_file_clean() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        let result = StatusFile::new(tmp.path(), "file.txt")
            .run(&ctx())
            .await
            .unwrap();
        assert_eq!(result.path, "file.txt");
        assert_eq!(result.status, "current");
    }

    #[tokio::test]
    async fn status_file_modified() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        fs::write(tmp.path().join("file.txt"), "changed").unwrap();
        let result = StatusFile::new(tmp.path(), "file.txt")
            .run(&ctx())
            .await
            .unwrap();
        assert!(result.status.contains("wt_modified"));
    }

    #[tokio::test]
    async fn status_list_detects_new_files() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        fs::write(tmp.path().join("new.txt"), "n").unwrap();
        let result = StatusList::new(tmp.path()).run(&ctx()).await.unwrap();
        assert!(result.entries.iter().any(|e| e.path == "new.txt"));
    }

    #[tokio::test]
    async fn should_ignore_gitignore() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        fs::write(tmp.path().join(".gitignore"), "*.log\n").unwrap();
        let result = StatusShouldIgnore::new(tmp.path(), "debug.log")
            .run(&ctx())
            .await
            .unwrap();
        assert!(result.ignored);
        let result = StatusShouldIgnore::new(tmp.path(), "file.txt")
            .run(&ctx())
            .await
            .unwrap();
        assert!(!result.ignored);
    }

    #[tokio::test]
    async fn execute_serializes_correctly() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        let value = StatusList::new(tmp.path()).execute(&ctx()).await.unwrap();
        assert!(value["entries"].is_array());
    }
}
