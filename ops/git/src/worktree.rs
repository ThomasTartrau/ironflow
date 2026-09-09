//! Worktree operations.

use std::path::PathBuf;

use async_trait::async_trait;
use git2::Repository;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::helpers::{blocking, to_value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeAddOutput {
    pub name: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeListOutput {
    pub worktrees: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeValidateOutput {
    pub name: String,
    pub valid: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreePruneOutput {
    pub name: String,
    pub pruned: bool,
}

/// Add a new worktree.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::worktree::WorktreeAdd;
/// use ironflow_core::operation::Operation;
///
/// let op = WorktreeAdd::new("/path/to/repo", "wt-feature", "/path/to/worktree");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct WorktreeAdd {
    repo_path: PathBuf,
    name: String,
    path: PathBuf,
}

impl WorktreeAdd {
    /// Create a new worktree-add operation.
    pub fn new(
        repo_path: impl Into<PathBuf>,
        name: impl Into<String>,
        path: impl Into<PathBuf>,
    ) -> Self {
        Self {
            repo_path: repo_path.into(),
            name: name.into(),
            path: path.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<WorktreeAddOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let name = self.name.clone();
        let path = self.path.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            repo.worktree(&name, &path, None)?;
            Ok(WorktreeAddOutput { name, path })
        })
        .await
    }
}

#[async_trait]
impl Operation for WorktreeAdd {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(
            serde_json::json!({ "repo_path": self.repo_path, "name": self.name, "path": self.path }),
        )
    }
}

impl TypedOperation for WorktreeAdd {
    type Output = WorktreeAddOutput;
}

/// List all worktrees.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::worktree::WorktreeList;
/// use ironflow_core::operation::Operation;
///
/// let op = WorktreeList::new("/path/to/repo");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct WorktreeList {
    repo_path: PathBuf,
}

impl WorktreeList {
    /// Create a new worktree-list operation.
    pub fn new(repo_path: impl Into<PathBuf>) -> Self {
        Self {
            repo_path: repo_path.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<WorktreeListOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let worktrees = repo.worktrees()?;
            let list = worktrees
                .iter()
                .filter_map(|w| w.map(String::from))
                .collect();
            Ok(WorktreeListOutput { worktrees: list })
        })
        .await
    }
}

#[async_trait]
impl Operation for WorktreeList {
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

impl TypedOperation for WorktreeList {
    type Output = WorktreeListOutput;
}

/// Validate a worktree.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::worktree::WorktreeValidate;
/// use ironflow_core::operation::Operation;
///
/// let op = WorktreeValidate::new("/path/to/repo", "wt-feature");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct WorktreeValidate {
    repo_path: PathBuf,
    name: String,
}

impl WorktreeValidate {
    /// Create a new worktree-validate operation.
    pub fn new(repo_path: impl Into<PathBuf>, name: impl Into<String>) -> Self {
        Self {
            repo_path: repo_path.into(),
            name: name.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(
        &self,
        _ctx: &OperationContext,
    ) -> Result<WorktreeValidateOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let name = self.name.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let wt = repo.find_worktree(&name)?;
            let valid = wt.validate().is_ok();
            Ok(WorktreeValidateOutput { name, valid })
        })
        .await
    }
}

#[async_trait]
impl Operation for WorktreeValidate {
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

impl TypedOperation for WorktreeValidate {
    type Output = WorktreeValidateOutput;
}

/// Prune a worktree.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::worktree::WorktreePrune;
/// use ironflow_core::operation::Operation;
///
/// let op = WorktreePrune::new("/path/to/repo", "wt-feature");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct WorktreePrune {
    repo_path: PathBuf,
    name: String,
}

impl WorktreePrune {
    /// Create a new worktree-prune operation.
    pub fn new(repo_path: impl Into<PathBuf>, name: impl Into<String>) -> Self {
        Self {
            repo_path: repo_path.into(),
            name: name.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(
        &self,
        _ctx: &OperationContext,
    ) -> Result<WorktreePruneOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let name = self.name.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let wt = repo.find_worktree(&name)?;
            wt.prune(None)?;
            Ok(WorktreePruneOutput { name, pruned: true })
        })
        .await
    }
}

#[async_trait]
impl Operation for WorktreePrune {
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

impl TypedOperation for WorktreePrune {
    type Output = WorktreePruneOutput;
}

#[cfg(test)]
mod tests {
    use std::fs;

    use ironflow_core::operation::Operation;

    use super::*;
    use crate::test_helpers::{ctx, init_repo};

    #[tokio::test]
    async fn add_and_list() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        let wt_path = tmp.path().join("wt-test");
        let result = WorktreeAdd::new(tmp.path(), "wt-test", &wt_path)
            .run(&ctx())
            .await
            .unwrap();
        assert_eq!(result.name, "wt-test");
        let list = WorktreeList::new(tmp.path()).run(&ctx()).await.unwrap();
        assert!(list.worktrees.contains(&"wt-test".to_string()));
    }

    #[tokio::test]
    async fn validate_existing_worktree() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        let wt_path = tmp.path().join("wt-val");
        WorktreeAdd::new(tmp.path(), "wt-val", &wt_path)
            .run(&ctx())
            .await
            .unwrap();
        let result = WorktreeValidate::new(tmp.path(), "wt-val")
            .run(&ctx())
            .await
            .unwrap();
        assert!(result.valid);
    }

    #[tokio::test]
    async fn prune_worktree() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        let wt_path = tmp.path().join("wt-prune");
        WorktreeAdd::new(tmp.path(), "wt-prune", &wt_path)
            .run(&ctx())
            .await
            .unwrap();
        fs::remove_dir_all(&wt_path).unwrap();
        let result = WorktreePrune::new(tmp.path(), "wt-prune")
            .run(&ctx())
            .await
            .unwrap();
        assert!(result.pruned);
    }

    #[tokio::test]
    async fn list_empty_worktrees() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        let result = WorktreeList::new(tmp.path()).run(&ctx()).await.unwrap();
        assert!(result.worktrees.is_empty());
    }

    #[tokio::test]
    async fn execute_serializes_correctly() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        let value = WorktreeList::new(tmp.path()).execute(&ctx()).await.unwrap();
        assert!(value["worktrees"].is_array());
    }
}
