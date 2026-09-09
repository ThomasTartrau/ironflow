//! Diff operations.

use std::path::PathBuf;

use async_trait::async_trait;
use git2::{ApplyLocation, Delta, Diff, Error, Oid, Repository};
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::helpers::{blocking, to_value};

/// A single delta entry in a diff.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffDelta {
    pub status: String,
    pub old_file: Option<String>,
    pub new_file: Option<String>,
}

/// Output of diff operations that return full diff info.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffOutput {
    pub files_changed: usize,
    pub insertions: usize,
    pub deletions: usize,
    pub deltas: Vec<DiffDelta>,
}

/// Output of [`DiffStats`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffStatsOutput {
    pub files_changed: usize,
    pub insertions: usize,
    pub deletions: usize,
}

/// Output of [`DiffApply`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffApplyOutput {
    pub applied: bool,
    pub to_index: bool,
}

fn delta_status_label(status: Delta) -> &'static str {
    match status {
        Delta::Unmodified => "unmodified",
        Delta::Added => "added",
        Delta::Deleted => "deleted",
        Delta::Modified => "modified",
        Delta::Renamed => "renamed",
        Delta::Copied => "copied",
        Delta::Ignored => "ignored",
        Delta::Untracked => "untracked",
        Delta::Typechange => "typechange",
        Delta::Unreadable => "unreadable",
        Delta::Conflicted => "conflicted",
    }
}

fn diff_to_output(diff: &Diff<'_>) -> Result<DiffOutput, Error> {
    let stats = diff.stats()?;
    let deltas: Vec<DiffDelta> = diff
        .deltas()
        .map(|delta| DiffDelta {
            status: delta_status_label(delta.status()).to_string(),
            old_file: delta
                .old_file()
                .path()
                .map(|p| p.to_string_lossy().into_owned()),
            new_file: delta
                .new_file()
                .path()
                .map(|p| p.to_string_lossy().into_owned()),
        })
        .collect();
    Ok(DiffOutput {
        files_changed: stats.files_changed(),
        insertions: stats.insertions(),
        deletions: stats.deletions(),
        deltas,
    })
}

/// Diff between two trees.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::diff::DiffTreeToTree;
/// use ironflow_core::operation::Operation;
///
/// let op = DiffTreeToTree::new("/path/to/repo", "abc123", "def456");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct DiffTreeToTree {
    repo_path: PathBuf,
    old_tree: String,
    new_tree: String,
}

impl DiffTreeToTree {
    /// Create a new tree-to-tree diff operation.
    pub fn new(
        repo_path: impl Into<PathBuf>,
        old_tree: impl Into<String>,
        new_tree: impl Into<String>,
    ) -> Self {
        Self {
            repo_path: repo_path.into(),
            old_tree: old_tree.into(),
            new_tree: new_tree.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<DiffOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let old = self.old_tree.clone();
        let new = self.new_tree.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let old_tree = repo.find_tree(Oid::from_str(&old)?)?;
            let new_tree = repo.find_tree(Oid::from_str(&new)?)?;
            let diff = repo.diff_tree_to_tree(Some(&old_tree), Some(&new_tree), None)?;
            diff_to_output(&diff)
        })
        .await
    }
}

#[async_trait]
impl Operation for DiffTreeToTree {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(
            serde_json::json!({ "repo_path": self.repo_path, "old_tree": self.old_tree, "new_tree": self.new_tree }),
        )
    }
}

impl TypedOperation for DiffTreeToTree {
    type Output = DiffOutput;
}

/// Diff between a tree and the index.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::diff::DiffTreeToIndex;
/// use ironflow_core::operation::Operation;
///
/// let op = DiffTreeToIndex::new("/path/to/repo", "abc123");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct DiffTreeToIndex {
    repo_path: PathBuf,
    tree_oid: String,
}

impl DiffTreeToIndex {
    /// Create a new tree-to-index diff operation.
    pub fn new(repo_path: impl Into<PathBuf>, tree_oid: impl Into<String>) -> Self {
        Self {
            repo_path: repo_path.into(),
            tree_oid: tree_oid.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<DiffOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let tree_oid = self.tree_oid.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let tree = repo.find_tree(Oid::from_str(&tree_oid)?)?;
            let diff = repo.diff_tree_to_index(Some(&tree), None, None)?;
            diff_to_output(&diff)
        })
        .await
    }
}

#[async_trait]
impl Operation for DiffTreeToIndex {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "repo_path": self.repo_path, "tree_oid": self.tree_oid }))
    }
}

impl TypedOperation for DiffTreeToIndex {
    type Output = DiffOutput;
}

/// Diff between the index and the working directory.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::diff::DiffIndexToWorkdir;
/// use ironflow_core::operation::Operation;
///
/// let op = DiffIndexToWorkdir::new("/path/to/repo");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct DiffIndexToWorkdir {
    repo_path: PathBuf,
}

impl DiffIndexToWorkdir {
    /// Create a new index-to-workdir diff operation.
    pub fn new(repo_path: impl Into<PathBuf>) -> Self {
        Self {
            repo_path: repo_path.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<DiffOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let diff = repo.diff_index_to_workdir(None, None)?;
            diff_to_output(&diff)
        })
        .await
    }
}

#[async_trait]
impl Operation for DiffIndexToWorkdir {
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

impl TypedOperation for DiffIndexToWorkdir {
    type Output = DiffOutput;
}

/// Get diff statistics.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::diff::DiffStats;
/// use ironflow_core::operation::Operation;
///
/// let op = DiffStats::new("/path/to/repo");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct DiffStats {
    repo_path: PathBuf,
}

impl DiffStats {
    /// Create a new diff-stats operation (index to workdir).
    pub fn new(repo_path: impl Into<PathBuf>) -> Self {
        Self {
            repo_path: repo_path.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<DiffStatsOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let diff = repo.diff_index_to_workdir(None, None)?;
            let stats = diff.stats()?;
            Ok(DiffStatsOutput {
                files_changed: stats.files_changed(),
                insertions: stats.insertions(),
                deletions: stats.deletions(),
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for DiffStats {
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

impl TypedOperation for DiffStats {
    type Output = DiffStatsOutput;
}

/// Find renamed/copied files in a diff.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::diff::DiffFindSimilar;
/// use ironflow_core::operation::Operation;
///
/// let op = DiffFindSimilar::new("/path/to/repo");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct DiffFindSimilar {
    repo_path: PathBuf,
}

impl DiffFindSimilar {
    /// Create a new find-similar operation.
    pub fn new(repo_path: impl Into<PathBuf>) -> Self {
        Self {
            repo_path: repo_path.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<DiffOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let mut diff = repo.diff_index_to_workdir(None, None)?;
            diff.find_similar(None)?;
            diff_to_output(&diff)
        })
        .await
    }
}

#[async_trait]
impl Operation for DiffFindSimilar {
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

impl TypedOperation for DiffFindSimilar {
    type Output = DiffOutput;
}

/// Apply a diff to the working directory or index.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::diff::DiffApply;
/// use ironflow_core::operation::Operation;
///
/// let op = DiffApply::new("/path/to/repo", true);
/// assert_eq!(op.kind(), "git");
/// ```
pub struct DiffApply {
    repo_path: PathBuf,
    to_index: bool,
}

impl DiffApply {
    /// Create a new diff-apply operation.
    ///
    /// If `to_index` is true, applies to the index. Otherwise, applies to the working directory.
    pub fn new(repo_path: impl Into<PathBuf>, to_index: bool) -> Self {
        Self {
            repo_path: repo_path.into(),
            to_index,
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<DiffApplyOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let to_index = self.to_index;
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let diff = repo.diff_index_to_workdir(None, None)?;
            let location = if to_index {
                ApplyLocation::Index
            } else {
                ApplyLocation::WorkDir
            };
            repo.apply(&diff, location, None)?;
            Ok(DiffApplyOutput {
                applied: true,
                to_index,
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for DiffApply {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "repo_path": self.repo_path, "to_index": self.to_index }))
    }
}

impl TypedOperation for DiffApply {
    type Output = DiffApplyOutput;
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use git2::{Repository, Signature};
    use ironflow_core::operation::Operation;

    use super::*;
    use crate::test_helpers::ctx;

    fn init_diff_repo(path: &Path) -> String {
        let repo = Repository::init(path).unwrap();
        fs::write(path.join("f.txt"), "original").unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_path(Path::new("f.txt")).unwrap();
        idx.write().unwrap();
        let tree_oid = idx.write_tree().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        let sig = Signature::now("Test", "t@t.com").unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();
        tree_oid.to_string()
    }

    #[tokio::test]
    async fn diff_stats_no_changes() {
        let tmp = tempfile::tempdir().unwrap();
        init_diff_repo(tmp.path());
        let result = DiffStats::new(tmp.path()).run(&ctx()).await.unwrap();
        assert_eq!(result.files_changed, 0);
        assert_eq!(result.insertions, 0);
        assert_eq!(result.deletions, 0);
    }

    #[tokio::test]
    async fn diff_stats_with_changes() {
        let tmp = tempfile::tempdir().unwrap();
        init_diff_repo(tmp.path());
        fs::write(tmp.path().join("f.txt"), "modified").unwrap();
        let result = DiffStats::new(tmp.path()).run(&ctx()).await.unwrap();
        assert!(result.files_changed > 0);
    }

    #[tokio::test]
    async fn diff_tree_to_tree() {
        let tmp = tempfile::tempdir().unwrap();
        let tree1_oid = init_diff_repo(tmp.path());
        let repo = Repository::open(tmp.path()).unwrap();
        let head = repo.head().unwrap().peel_to_commit().unwrap();

        fs::write(tmp.path().join("f.txt"), "v2").unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_path(Path::new("f.txt")).unwrap();
        idx.write().unwrap();
        let tree2_oid = idx.write_tree().unwrap();
        let tree2 = repo.find_tree(tree2_oid).unwrap();
        let sig = Signature::now("Test", "t@t.com").unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "second", &tree2, &[&head])
            .unwrap();

        let result = DiffTreeToTree::new(tmp.path(), &tree1_oid, tree2_oid.to_string())
            .run(&ctx())
            .await
            .unwrap();
        assert!(result.files_changed > 0);
        assert!(!result.deltas.is_empty());
    }

    #[tokio::test]
    async fn diff_tree_to_index() {
        let tmp = tempfile::tempdir().unwrap();
        let tree_oid = init_diff_repo(tmp.path());
        fs::write(tmp.path().join("f.txt"), "staged").unwrap();
        let repo = Repository::open(tmp.path()).unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_path(Path::new("f.txt")).unwrap();
        idx.write().unwrap();
        let result = DiffTreeToIndex::new(tmp.path(), &tree_oid)
            .run(&ctx())
            .await
            .unwrap();
        assert!(result.files_changed > 0);
    }

    #[tokio::test]
    async fn find_similar_empty() {
        let tmp = tempfile::tempdir().unwrap();
        init_diff_repo(tmp.path());
        let result = DiffFindSimilar::new(tmp.path()).run(&ctx()).await.unwrap();
        assert_eq!(result.files_changed, 0);
    }

    #[tokio::test]
    async fn execute_serializes_correctly() {
        let tmp = tempfile::tempdir().unwrap();
        init_diff_repo(tmp.path());
        let value = DiffStats::new(tmp.path()).execute(&ctx()).await.unwrap();
        assert_eq!(value["files_changed"], 0);
    }
}
