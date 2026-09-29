//! Worktree operations.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use git2::{Error, Repository, WorktreeAddOptions, WorktreePruneOptions};
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::helpers::{blocking, to_value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeAddOutput {
    pub name: String,
    pub path: PathBuf,
    /// SHA the worktree `HEAD` is detached at, when added with
    /// [`WorktreeAdd::detached_at`].
    pub detached_at: Option<String>,
}

/// Output of [`WorktreeRemove`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeRemoveOutput {
    /// Name of the worktree.
    pub name: String,
    /// `false` when no worktree of that name existed.
    pub removed: bool,
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
    detached_at: Option<String>,
}

impl WorktreeAdd {
    /// Create a new worktree-add operation.
    ///
    /// The worktree is checked out on a new branch named `name`, created from
    /// the repository `HEAD`, unless [`WorktreeAdd::detached_at`] is set.
    pub fn new(
        repo_path: impl Into<PathBuf>,
        name: impl Into<String>,
        path: impl Into<PathBuf>,
    ) -> Self {
        Self {
            repo_path: repo_path.into(),
            name: name.into(),
            path: path.into(),
            detached_at: None,
        }
    }

    /// Check the worktree out with a detached `HEAD` at `commit` (a SHA or
    /// any revspec), without creating a branch.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_git::worktree::WorktreeAdd;
    ///
    /// let op = WorktreeAdd::new("/srv/repo.git", "mr-42", "/tmp/mr-42")
    ///     .detached_at("5f1c0e2a9b7d4c3e8f6a1b2c3d4e5f60718293a4");
    /// ```
    pub fn detached_at(mut self, commit: impl Into<String>) -> Self {
        self.detached_at = Some(commit.into());
        self
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] if the repository cannot be
    /// opened, the commit does not resolve, or the worktree cannot be added.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<WorktreeAddOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let name = self.name.clone();
        let path = self.path.clone();
        let detached_at = self.detached_at.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let detached_at = match detached_at {
                Some(spec) => Some(add_detached(&repo, &name, &path, &spec)?),
                None => {
                    repo.worktree(&name, &path, None)?;
                    None
                }
            };
            Ok(WorktreeAddOutput {
                name,
                path,
                detached_at,
            })
        })
        .await
    }
}

/// Add a worktree with a detached `HEAD` at `spec` and return the SHA.
///
/// libgit2 only adds a worktree on a branch, so the worktree is added on a
/// temporary branch pointing at the commit, its `HEAD` is detached, then the
/// branch is deleted, on success and on failure alike.
fn add_detached(repo: &Repository, name: &str, path: &Path, spec: &str) -> Result<String, Error> {
    let commit = repo.revparse_single(spec)?.peel_to_commit()?;
    let mut temp = repo.branch(&format!("ironflow-detached/{name}"), &commit, true)?;
    let added = (|| {
        let mut opts = WorktreeAddOptions::new();
        opts.reference(Some(temp.get()));
        let worktree = repo.worktree(name, path, Some(&opts))?;
        Repository::open_from_worktree(&worktree)?.set_head_detached(commit.id())
    })();
    let deleted = temp.delete();
    added?;
    deleted?;
    Ok(commit.id().to_string())
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
            serde_json::json!({ "repo_path": self.repo_path, "name": self.name, "path": self.path, "detached_at": self.detached_at }),
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

/// Remove a worktree: delete its directory and prune its entry.
///
/// Idempotent, so a retried step does not fail: an unknown worktree name
/// returns `removed: false`, and a worktree whose directory is already gone
/// only has its entry pruned. A locked worktree is not removed.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::worktree::WorktreeRemove;
/// use ironflow_core::operation::Operation;
///
/// let op = WorktreeRemove::new("/path/to/repo", "wt-feature");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct WorktreeRemove {
    repo_path: PathBuf,
    name: String,
}

impl WorktreeRemove {
    /// Create a new worktree-remove operation.
    pub fn new(repo_path: impl Into<PathBuf>, name: impl Into<String>) -> Self {
        Self {
            repo_path: repo_path.into(),
            name: name.into(),
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] if the repository cannot be
    /// opened, or the worktree is locked or cannot be deleted.
    pub async fn run(
        &self,
        _ctx: &OperationContext,
    ) -> Result<WorktreeRemoveOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let name = self.name.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let exists = repo.worktrees()?.iter().flatten().any(|n| n == name);
            if !exists {
                return Ok(WorktreeRemoveOutput {
                    name,
                    removed: false,
                });
            }
            let mut opts = WorktreePruneOptions::new();
            opts.valid(true).working_tree(true);
            repo.find_worktree(&name)?.prune(Some(&mut opts))?;
            Ok(WorktreeRemoveOutput {
                name,
                removed: true,
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for WorktreeRemove {
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

impl TypedOperation for WorktreeRemove {
    type Output = WorktreeRemoveOutput;
}

#[cfg(test)]
mod tests {
    use std::fs;

    use git2::BranchType;
    use ironflow_core::operation::Operation;

    use super::*;
    use crate::test_helpers::{ctx, init_repo, make_two_commits};

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
    async fn add_detached_at_commit_checks_out_sha_without_creating_branch() {
        let tmp = tempfile::tempdir().unwrap();
        let (first, _second) = make_two_commits(tmp.path());
        let wt_path = tmp.path().join("wt-review");

        let result = WorktreeAdd::new(tmp.path(), "wt-review", &wt_path)
            .detached_at(&first)
            .run(&ctx())
            .await
            .unwrap();

        assert_eq!(result.detached_at.as_deref(), Some(first.as_str()));
        let wt_repo = Repository::open(&wt_path).unwrap();
        assert!(wt_repo.head_detached().unwrap());
        assert_eq!(wt_repo.head().unwrap().target().unwrap().to_string(), first);
        // `other.txt` only exists in the second commit.
        assert!(wt_path.join("file.txt").exists());
        assert!(!wt_path.join("other.txt").exists());
        let repo = Repository::open(tmp.path()).unwrap();
        let branches: Vec<String> = repo
            .branches(Some(BranchType::Local))
            .unwrap()
            .map(|b| b.unwrap().0.name().unwrap().unwrap().to_string())
            .collect();
        assert_eq!(branches.len(), 1, "unexpected branches: {branches:?}");
    }

    #[tokio::test]
    async fn add_detached_at_accepts_a_revspec() {
        let tmp = tempfile::tempdir().unwrap();
        let (first, _second) = make_two_commits(tmp.path());
        let wt_path = tmp.path().join("wt-rev");

        let result = WorktreeAdd::new(tmp.path(), "wt-rev", &wt_path)
            .detached_at("HEAD~1")
            .run(&ctx())
            .await
            .unwrap();

        assert_eq!(result.detached_at.as_deref(), Some(first.as_str()));
    }

    #[tokio::test]
    async fn add_detached_at_unknown_commit_fails_and_leaves_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        make_two_commits(tmp.path());
        let wt_path = tmp.path().join("wt-bad");

        let result = WorktreeAdd::new(tmp.path(), "wt-bad", &wt_path)
            .detached_at("0123456789abcdef0123456789abcdef01234567")
            .run(&ctx())
            .await;

        assert!(result.is_err());
        assert!(!wt_path.exists());
        let repo = Repository::open(tmp.path()).unwrap();
        assert_eq!(repo.branches(Some(BranchType::Local)).unwrap().count(), 1);
        assert!(repo.worktrees().unwrap().is_empty());
    }

    #[tokio::test]
    async fn add_detached_on_existing_worktree_name_fails_and_leaves_no_branch() {
        let tmp = tempfile::tempdir().unwrap();
        let (first, _second) = make_two_commits(tmp.path());
        WorktreeAdd::new(tmp.path(), "wt-dup", tmp.path().join("wt-dup"))
            .detached_at(&first)
            .run(&ctx())
            .await
            .unwrap();

        let result = WorktreeAdd::new(tmp.path(), "wt-dup", tmp.path().join("wt-dup-2"))
            .detached_at(&first)
            .run(&ctx())
            .await;

        assert!(result.is_err());
        let repo = Repository::open(tmp.path()).unwrap();
        assert_eq!(repo.branches(Some(BranchType::Local)).unwrap().count(), 1);
    }

    #[tokio::test]
    async fn worktree_remove_deletes_directory_and_entry() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        let wt_path = tmp.path().join("wt-rm");
        WorktreeAdd::new(tmp.path(), "wt-rm", &wt_path)
            .run(&ctx())
            .await
            .unwrap();

        let result = WorktreeRemove::new(tmp.path(), "wt-rm")
            .run(&ctx())
            .await
            .unwrap();

        assert!(result.removed);
        assert!(!wt_path.exists());
        let list = WorktreeList::new(tmp.path()).run(&ctx()).await.unwrap();
        assert!(list.worktrees.is_empty());
    }

    #[tokio::test]
    async fn worktree_remove_after_directory_deleted_prunes_entry() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        let wt_path = tmp.path().join("wt-gone");
        WorktreeAdd::new(tmp.path(), "wt-gone", &wt_path)
            .run(&ctx())
            .await
            .unwrap();
        fs::remove_dir_all(&wt_path).unwrap();

        let result = WorktreeRemove::new(tmp.path(), "wt-gone")
            .run(&ctx())
            .await
            .unwrap();

        assert!(result.removed);
        let list = WorktreeList::new(tmp.path()).run(&ctx()).await.unwrap();
        assert!(list.worktrees.is_empty());
    }

    #[tokio::test]
    async fn worktree_remove_unknown_name_is_a_noop() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());

        let result = WorktreeRemove::new(tmp.path(), "never-added")
            .run(&ctx())
            .await
            .unwrap();

        assert!(!result.removed);
    }

    #[tokio::test]
    async fn worktree_remove_twice_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        let wt_path = tmp.path().join("wt-twice");
        WorktreeAdd::new(tmp.path(), "wt-twice", &wt_path)
            .run(&ctx())
            .await
            .unwrap();
        let remove = WorktreeRemove::new(tmp.path(), "wt-twice");

        assert!(remove.run(&ctx()).await.unwrap().removed);
        assert!(!remove.run(&ctx()).await.unwrap().removed);
    }

    #[tokio::test]
    async fn worktree_remove_on_missing_repo_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let result = WorktreeRemove::new(tmp.path().join("nope"), "wt")
            .run(&ctx())
            .await;
        assert!(result.is_err());
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
