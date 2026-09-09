//! Merge operations.

use std::path::PathBuf;

use async_trait::async_trait;
use git2::{BranchType, MergeOptions, Oid, Repository};
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::helpers::{blocking, to_value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeBranchOutput {
    pub branch: String,
    pub merged: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeAnalysisOutput {
    pub up_to_date: bool,
    pub fast_forward: bool,
    pub normal: bool,
    pub none: bool,
    pub no_fast_forward: bool,
    pub fastforward_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeConflictOutput {
    pub has_conflicts: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeBaseOutput {
    pub merge_base: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeCleanupOutput {
    pub cleaned: bool,
}

/// Merge a branch into HEAD.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::merge::MergeBranch;
/// use ironflow_core::operation::Operation;
///
/// let op = MergeBranch::new("/path/to/repo", "feature");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct MergeBranch {
    repo_path: PathBuf,
    branch_name: String,
}

impl MergeBranch {
    /// Create a new merge operation.
    pub fn new(repo_path: impl Into<PathBuf>, branch_name: impl Into<String>) -> Self {
        Self {
            repo_path: repo_path.into(),
            branch_name: branch_name.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<MergeBranchOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let branch_name = self.branch_name.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let reference = repo.find_branch(&branch_name, BranchType::Local)?;
            let commit = reference.get().peel_to_commit()?;
            let annotated = repo.find_annotated_commit(commit.id())?;
            repo.merge(&[&annotated], Some(&mut MergeOptions::new()), None)?;
            Ok(MergeBranchOutput {
                branch: branch_name,
                merged: true,
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for MergeBranch {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "repo_path": self.repo_path, "branch": self.branch_name }))
    }
}

impl TypedOperation for MergeBranch {
    type Output = MergeBranchOutput;
}

/// Analyze what kind of merge is needed.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::merge::MergeAnalysisOp;
/// use ironflow_core::operation::Operation;
///
/// let op = MergeAnalysisOp::new("/path/to/repo", "abc123");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct MergeAnalysisOp {
    repo_path: PathBuf,
    oid: String,
}

impl MergeAnalysisOp {
    /// Create a new merge-analysis operation.
    pub fn new(repo_path: impl Into<PathBuf>, oid: impl Into<String>) -> Self {
        Self {
            repo_path: repo_path.into(),
            oid: oid.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(
        &self,
        _ctx: &OperationContext,
    ) -> Result<MergeAnalysisOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let oid_str = self.oid.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let oid = Oid::from_str(&oid_str)?;
            let annotated = repo.find_annotated_commit(oid)?;
            let (analysis, preference) = repo.merge_analysis(&[&annotated])?;
            Ok(MergeAnalysisOutput {
                up_to_date: analysis.is_up_to_date(),
                fast_forward: analysis.is_fast_forward(),
                normal: analysis.is_normal(),
                none: analysis.is_none(),
                no_fast_forward: preference.is_no_fast_forward(),
                fastforward_only: preference.is_fastforward_only(),
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for MergeAnalysisOp {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "repo_path": self.repo_path, "oid": self.oid }))
    }
}

impl TypedOperation for MergeAnalysisOp {
    type Output = MergeAnalysisOutput;
}

/// Merge two commits as trees (without touching the working directory).
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::merge::MergeCommits;
/// use ironflow_core::operation::Operation;
///
/// let op = MergeCommits::new("/path/to/repo", "abc123", "def456");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct MergeCommits {
    repo_path: PathBuf,
    our_oid: String,
    their_oid: String,
}

impl MergeCommits {
    /// Create a new merge-commits operation.
    pub fn new(
        repo_path: impl Into<PathBuf>,
        our_oid: impl Into<String>,
        their_oid: impl Into<String>,
    ) -> Self {
        Self {
            repo_path: repo_path.into(),
            our_oid: our_oid.into(),
            their_oid: their_oid.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(
        &self,
        _ctx: &OperationContext,
    ) -> Result<MergeConflictOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let our = self.our_oid.clone();
        let their = self.their_oid.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let our_commit = repo.find_commit(Oid::from_str(&our)?)?;
            let their_commit = repo.find_commit(Oid::from_str(&their)?)?;
            let index = repo.merge_commits(&our_commit, &their_commit, None)?;
            Ok(MergeConflictOutput {
                has_conflicts: index.has_conflicts(),
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for MergeCommits {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(
            serde_json::json!({ "repo_path": self.repo_path, "ours": self.our_oid, "theirs": self.their_oid }),
        )
    }
}

impl TypedOperation for MergeCommits {
    type Output = MergeConflictOutput;
}

/// Find the merge base between two commits.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::merge::MergeBaseOp;
/// use ironflow_core::operation::Operation;
///
/// let op = MergeBaseOp::new("/path/to/repo", "abc123", "def456");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct MergeBaseOp {
    repo_path: PathBuf,
    one: String,
    two: String,
}

impl MergeBaseOp {
    /// Create a new merge-base operation.
    pub fn new(
        repo_path: impl Into<PathBuf>,
        one: impl Into<String>,
        two: impl Into<String>,
    ) -> Self {
        Self {
            repo_path: repo_path.into(),
            one: one.into(),
            two: two.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<MergeBaseOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let one = self.one.clone();
        let two = self.two.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let oid1 = Oid::from_str(&one)?;
            let oid2 = Oid::from_str(&two)?;
            let base = repo.merge_base(oid1, oid2)?;
            Ok(MergeBaseOutput {
                merge_base: base.to_string(),
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for MergeBaseOp {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "repo_path": self.repo_path, "one": self.one, "two": self.two }))
    }
}

impl TypedOperation for MergeBaseOp {
    type Output = MergeBaseOutput;
}

/// Clean up merge state files.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::merge::MergeCleanupState;
/// use ironflow_core::operation::Operation;
///
/// let op = MergeCleanupState::new("/path/to/repo");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct MergeCleanupState {
    repo_path: PathBuf,
}

impl MergeCleanupState {
    /// Create a new cleanup-state operation.
    pub fn new(repo_path: impl Into<PathBuf>) -> Self {
        Self {
            repo_path: repo_path.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<MergeCleanupOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            repo.cleanup_state()?;
            Ok(MergeCleanupOutput { cleaned: true })
        })
        .await
    }
}

#[async_trait]
impl Operation for MergeCleanupState {
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

impl TypedOperation for MergeCleanupState {
    type Output = MergeCleanupOutput;
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use git2::{Repository, Signature};
    use ironflow_core::operation::Operation;

    use super::*;
    use crate::test_helpers::ctx;

    fn init_with_branch(path: &Path) -> (String, String) {
        use git2::build::CheckoutBuilder;
        let repo = Repository::init(path).unwrap();
        let sig = Signature::now("Test", "t@t.com").unwrap();
        fs::write(path.join("f.txt"), "base").unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_path(Path::new("f.txt")).unwrap();
        idx.write().unwrap();
        let tree = repo.find_tree(idx.write_tree().unwrap()).unwrap();
        let c1 = repo
            .commit(Some("HEAD"), &sig, &sig, "base", &tree, &[])
            .unwrap();
        let base = repo.find_commit(c1).unwrap();
        repo.branch("feature", &base, false).unwrap();

        let mut tb = repo.treebuilder(Some(&tree)).unwrap();
        let blob_oid = repo.blob(b"feature-only").unwrap();
        tb.insert("g.txt", blob_oid, 0o100644).unwrap();
        let feat_tree = repo.find_tree(tb.write().unwrap()).unwrap();
        repo.commit(
            Some("refs/heads/feature"),
            &sig,
            &sig,
            "feature commit",
            &feat_tree,
            &[&base],
        )
        .unwrap();

        repo.checkout_head(Some(CheckoutBuilder::new().force()))
            .unwrap();
        let head_oid = repo
            .head()
            .unwrap()
            .peel_to_commit()
            .unwrap()
            .id()
            .to_string();
        let feat_oid = repo
            .find_branch("feature", BranchType::Local)
            .unwrap()
            .get()
            .peel_to_commit()
            .unwrap()
            .id()
            .to_string();
        (head_oid, feat_oid)
    }

    #[tokio::test]
    async fn merge_analysis_fast_forward() {
        let tmp = tempfile::tempdir().unwrap();
        let (_, feat_oid) = init_with_branch(tmp.path());
        let result = MergeAnalysisOp::new(tmp.path(), &feat_oid)
            .run(&ctx())
            .await
            .unwrap();
        assert!(result.fast_forward);
        assert!(!result.up_to_date);
    }

    #[tokio::test]
    async fn merge_base_found() {
        let tmp = tempfile::tempdir().unwrap();
        let (head_oid, feat_oid) = init_with_branch(tmp.path());
        let result = MergeBaseOp::new(tmp.path(), &head_oid, &feat_oid)
            .run(&ctx())
            .await
            .unwrap();
        assert_eq!(result.merge_base, head_oid);
    }

    #[tokio::test]
    async fn merge_branch_succeeds() {
        let tmp = tempfile::tempdir().unwrap();
        init_with_branch(tmp.path());
        let result = MergeBranch::new(tmp.path(), "feature")
            .run(&ctx())
            .await
            .unwrap();
        assert!(result.merged);
        assert_eq!(result.branch, "feature");
    }

    #[tokio::test]
    async fn merge_commits_no_conflict() {
        let tmp = tempfile::tempdir().unwrap();
        let (head_oid, feat_oid) = init_with_branch(tmp.path());
        let result = MergeCommits::new(tmp.path(), &head_oid, &feat_oid)
            .run(&ctx())
            .await
            .unwrap();
        assert!(!result.has_conflicts);
    }

    #[tokio::test]
    async fn cleanup_state() {
        let tmp = tempfile::tempdir().unwrap();
        init_with_branch(tmp.path());
        let result = MergeCleanupState::new(tmp.path())
            .run(&ctx())
            .await
            .unwrap();
        assert!(result.cleaned);
    }

    #[tokio::test]
    async fn execute_serializes_correctly() {
        let tmp = tempfile::tempdir().unwrap();
        let (_, feat_oid) = init_with_branch(tmp.path());
        let value = MergeAnalysisOp::new(tmp.path(), &feat_oid)
            .execute(&ctx())
            .await
            .unwrap();
        assert!(value["fast_forward"].as_bool().unwrap());
    }
}
