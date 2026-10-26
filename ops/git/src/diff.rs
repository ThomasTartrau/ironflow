//! Diff operations.

use std::path::PathBuf;

use async_trait::async_trait;
use git2::{ApplyLocation, Delta, Diff, DiffOptions, Error, Oid, Repository};
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::helpers::{blocking, to_value};
use crate::refname::check_ref_name;

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

/// Output of [`DiffAgainstRef`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffAgainstRefOutput {
    /// The base ref, as given.
    pub base_ref: String,
    /// The commit where HEAD forked from the base ref.
    pub merge_base: String,
    /// Paths changed since the merge base, sorted, without duplicates.
    pub files: Vec<String>,
}

/// List the files changed since HEAD forked from a ref given by name.
///
/// The base ref is a short name such as `main` or `origin/main`. The list
/// covers what is committed, staged, unstaged and untracked since the merge
/// base of HEAD and the base ref (ignored files excluded). Paths are raw:
/// never quoted or escaped as `git diff --name-only` would. A rename shows
/// as the old path and the new path, both listed. `repo_path` may be a
/// worktree.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::diff::DiffAgainstRef;
/// use ironflow_core::operation::Operation;
///
/// let op = DiffAgainstRef::new("/path/to/repo", "origin/main");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct DiffAgainstRef {
    repo_path: PathBuf,
    base_ref: String,
}

impl DiffAgainstRef {
    /// Create a new diff-against-ref operation.
    pub fn new(repo_path: impl Into<PathBuf>, base_ref: impl Into<String>) -> Self {
        Self {
            repo_path: repo_path.into(),
            base_ref: base_ref.into(),
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] if the base ref is not a valid ref
    /// name (checked before the repository is opened), the repository cannot
    /// be opened, the base ref does not resolve, HEAD is unborn, or HEAD and
    /// the base ref share no history.
    pub async fn run(
        &self,
        _ctx: &OperationContext,
    ) -> Result<DiffAgainstRefOutput, OperationError> {
        check_ref_name("base ref", &self.base_ref)?;
        let repo_path = self.repo_path.clone();
        let base_ref = self.base_ref.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let base = repo
                .resolve_reference_from_short_name(&base_ref)?
                .peel_to_commit()?;
            let head = repo.head()?.peel_to_commit()?;
            let merge_base = repo.merge_base(head.id(), base.id())?;
            let tree = repo.find_commit(merge_base)?.tree()?;
            let mut opts = DiffOptions::new();
            opts.include_untracked(true).recurse_untracked_dirs(true);
            let diff = repo.diff_tree_to_workdir_with_index(Some(&tree), Some(&mut opts))?;
            let mut files: Vec<String> = diff
                .deltas()
                .filter_map(|delta| {
                    let file = if delta.status() == Delta::Deleted {
                        delta.old_file()
                    } else {
                        delta.new_file()
                    };
                    file.path().map(|p| p.to_string_lossy().into_owned())
                })
                .collect();
            files.sort();
            files.dedup();
            Ok(DiffAgainstRefOutput {
                base_ref,
                merge_base: merge_base.to_string(),
                files,
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for DiffAgainstRef {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "repo_path": self.repo_path, "base_ref": self.base_ref }))
    }
}

impl TypedOperation for DiffAgainstRef {
    type Output = DiffAgainstRefOutput;
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use git2::{Repository, Signature};
    use ironflow_core::operation::Operation;

    use super::*;
    use crate::test_helpers::{ctx, init_repo};

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

    /// Fork `base` and `feature` off the init commit, `feature` checked out.
    fn fork_repo(path: &Path) -> (Repository, Oid) {
        let init = init_repo(path);
        let repo = Repository::open(path).unwrap();
        let commit = repo.find_commit(init).unwrap();
        repo.branch("base", &commit, false).unwrap();
        repo.branch("feature", &commit, false).unwrap();
        repo.set_head("refs/heads/feature").unwrap();
        (repo, init)
    }

    #[tokio::test]
    async fn diff_against_ref_lists_untracked_files_since_merge_base() {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, init) = fork_repo(tmp.path());
        let sig = Signature::now("Test", "t@t.com").unwrap();
        let init_commit = repo.find_commit(init).unwrap();

        // A commit on base after the fork must not show up.
        let mut builder = repo
            .treebuilder(Some(&init_commit.tree().unwrap()))
            .unwrap();
        let blob = repo.blob(b"later").unwrap();
        builder.insert("later.txt", blob, 0o100644).unwrap();
        let later_tree = repo.find_tree(builder.write().unwrap()).unwrap();
        repo.commit(
            Some("refs/heads/base"),
            &sig,
            &sig,
            "later",
            &later_tree,
            &[&init_commit],
        )
        .unwrap();

        fs::write(tmp.path().join("committed.txt"), "c").unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_path(Path::new("committed.txt")).unwrap();
        idx.write().unwrap();
        let tree = repo.find_tree(idx.write_tree().unwrap()).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "feature", &tree, &[&init_commit])
            .unwrap();

        fs::write(tmp.path().join("file.txt"), "changed").unwrap();
        fs::create_dir(tmp.path().join("dir")).unwrap();
        fs::write(tmp.path().join("dir/é file.txt"), "u").unwrap();

        let out = DiffAgainstRef::new(tmp.path(), "base")
            .run(&ctx())
            .await
            .unwrap();
        assert_eq!(out.base_ref, "base");
        assert_eq!(out.merge_base, init.to_string());
        assert_eq!(out.files, ["committed.txt", "dir/é file.txt", "file.txt"]);
    }

    #[tokio::test]
    async fn diff_against_ref_resolves_remote_tracking_name() {
        let tmp = tempfile::tempdir().unwrap();
        let init = init_repo(tmp.path());
        let repo = Repository::open(tmp.path()).unwrap();
        repo.reference("refs/remotes/origin/main", init, false, "test")
            .unwrap();
        fs::write(tmp.path().join("new.txt"), "n").unwrap();
        let out = DiffAgainstRef::new(tmp.path(), "origin/main")
            .run(&ctx())
            .await
            .unwrap();
        assert_eq!(out.merge_base, init.to_string());
        assert_eq!(out.files, ["new.txt"]);
    }

    #[tokio::test]
    async fn diff_against_ref_rejects_invalid_ref() {
        let missing = tempfile::tempdir().unwrap().path().join("nope");
        for name in ["-x", "a..b", "x y", ""] {
            let err = DiffAgainstRef::new(&missing, name)
                .run(&ctx())
                .await
                .unwrap_err()
                .to_string();
            assert!(err.contains("invalid base ref name"), "got: {err}");
        }
    }

    #[tokio::test]
    async fn diff_against_ref_unborn_head_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let init = init_repo(tmp.path());
        let repo = Repository::open(tmp.path()).unwrap();
        repo.branch("base", &repo.find_commit(init).unwrap(), false)
            .unwrap();
        repo.set_head("refs/heads/unborn").unwrap();
        let result = DiffAgainstRef::new(tmp.path(), "base").run(&ctx()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn diff_against_ref_unrelated_histories_errors() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        let repo = Repository::open(tmp.path()).unwrap();
        let sig = Signature::now("Test", "t@t.com").unwrap();
        let empty = repo
            .find_tree(repo.treebuilder(None).unwrap().write().unwrap())
            .unwrap();
        repo.commit(Some("refs/heads/other"), &sig, &sig, "root", &empty, &[])
            .unwrap();
        let result = DiffAgainstRef::new(tmp.path(), "other").run(&ctx()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn diff_against_ref_unknown_ref_errors() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        let result = DiffAgainstRef::new(tmp.path(), "no-such-ref")
            .run(&ctx())
            .await;
        assert!(result.is_err());
    }
}
