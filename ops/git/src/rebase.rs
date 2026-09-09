//! Rebase operations.

use std::path::PathBuf;

use async_trait::async_trait;
use git2::{BranchType, RebaseOperationType, Repository, Signature};
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::helpers::{blocking, to_value};

fn rebase_op_label(kind: Option<RebaseOperationType>) -> &'static str {
    match kind {
        Some(RebaseOperationType::Pick) => "pick",
        Some(RebaseOperationType::Reword) => "reword",
        Some(RebaseOperationType::Edit) => "edit",
        Some(RebaseOperationType::Squash) => "squash",
        Some(RebaseOperationType::Fixup) => "fixup",
        Some(RebaseOperationType::Exec) => "exec",
        None => "unknown",
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RebaseInitOutput {
    pub branch: String,
    pub upstream: String,
    pub operations: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RebaseNextOutput {
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub op_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RebaseCommitOutput {
    pub oid: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RebaseAbortOutput {
    pub aborted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RebaseFinishOutput {
    pub finished: bool,
}

/// Start a rebase operation.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::rebase::RebaseInit;
/// use ironflow_core::operation::Operation;
///
/// let op = RebaseInit::new("/path/to/repo", "feature", "main");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct RebaseInit {
    repo_path: PathBuf,
    branch: String,
    upstream: String,
}

impl RebaseInit {
    /// Create a new rebase-init operation.
    pub fn new(
        repo_path: impl Into<PathBuf>,
        branch: impl Into<String>,
        upstream: impl Into<String>,
    ) -> Self {
        Self {
            repo_path: repo_path.into(),
            branch: branch.into(),
            upstream: upstream.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<RebaseInitOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let branch = self.branch.clone();
        let upstream = self.upstream.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let branch_ref = repo.find_branch(&branch, BranchType::Local)?;
            let branch_commit = branch_ref.get().peel_to_commit()?;
            let branch_annotated = repo.find_annotated_commit(branch_commit.id())?;
            let upstream_ref = repo.find_branch(&upstream, BranchType::Local)?;
            let upstream_commit = upstream_ref.get().peel_to_commit()?;
            let upstream_annotated = repo.find_annotated_commit(upstream_commit.id())?;
            let rebase = repo.rebase(
                Some(&branch_annotated),
                Some(&upstream_annotated),
                None,
                None,
            )?;
            let count = rebase.len();
            Ok(RebaseInitOutput {
                branch,
                upstream,
                operations: count,
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for RebaseInit {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(
            serde_json::json!({ "repo_path": self.repo_path, "branch": self.branch, "upstream": self.upstream }),
        )
    }
}

impl TypedOperation for RebaseInit {
    type Output = RebaseInitOutput;
}

/// Apply the next rebase operation.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::rebase::RebaseNext;
/// use ironflow_core::operation::Operation;
///
/// let op = RebaseNext::new("/path/to/repo");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct RebaseNext {
    repo_path: PathBuf,
}

impl RebaseNext {
    /// Create a new rebase-next operation.
    pub fn new(repo_path: impl Into<PathBuf>) -> Self {
        Self {
            repo_path: repo_path.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<RebaseNextOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let mut rebase = repo.open_rebase(None)?;
            let op = rebase.next();
            match op {
                Some(Ok(operation)) => Ok(RebaseNextOutput {
                    op_type: Some(rebase_op_label(operation.kind()).to_string()),
                    id: Some(operation.id().to_string()),
                    has_more: true,
                }),
                Some(Err(e)) => Err(e),
                None => Ok(RebaseNextOutput {
                    op_type: None,
                    id: None,
                    has_more: false,
                }),
            }
        })
        .await
    }
}

#[async_trait]
impl Operation for RebaseNext {
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

impl TypedOperation for RebaseNext {
    type Output = RebaseNextOutput;
}

/// Commit the current rebase operation.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::rebase::RebaseCommit;
/// use ironflow_core::operation::Operation;
///
/// let op = RebaseCommit::new("/path/to/repo", "Author", "author@example.com");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct RebaseCommit {
    repo_path: PathBuf,
    author_name: String,
    author_email: String,
}

impl RebaseCommit {
    /// Create a new rebase-commit operation.
    pub fn new(
        repo_path: impl Into<PathBuf>,
        author_name: impl Into<String>,
        author_email: impl Into<String>,
    ) -> Self {
        Self {
            repo_path: repo_path.into(),
            author_name: author_name.into(),
            author_email: author_email.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<RebaseCommitOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let name = self.author_name.clone();
        let email = self.author_email.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let mut rebase = repo.open_rebase(None)?;
            let sig = Signature::now(&name, &email)?;
            let oid = rebase.commit(None, &sig, None)?;
            Ok(RebaseCommitOutput {
                oid: oid.to_string(),
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for RebaseCommit {
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

impl TypedOperation for RebaseCommit {
    type Output = RebaseCommitOutput;
}

/// Abort a rebase in progress.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::rebase::RebaseAbort;
/// use ironflow_core::operation::Operation;
///
/// let op = RebaseAbort::new("/path/to/repo");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct RebaseAbort {
    repo_path: PathBuf,
}

impl RebaseAbort {
    /// Create a new rebase-abort operation.
    pub fn new(repo_path: impl Into<PathBuf>) -> Self {
        Self {
            repo_path: repo_path.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<RebaseAbortOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let mut rebase = repo.open_rebase(None)?;
            rebase.abort()?;
            Ok(RebaseAbortOutput { aborted: true })
        })
        .await
    }
}

#[async_trait]
impl Operation for RebaseAbort {
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

impl TypedOperation for RebaseAbort {
    type Output = RebaseAbortOutput;
}

/// Finish a rebase.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::rebase::RebaseFinish;
/// use ironflow_core::operation::Operation;
///
/// let op = RebaseFinish::new("/path/to/repo", "Author", "author@example.com");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct RebaseFinish {
    repo_path: PathBuf,
    author_name: String,
    author_email: String,
}

impl RebaseFinish {
    /// Create a new rebase-finish operation.
    pub fn new(
        repo_path: impl Into<PathBuf>,
        author_name: impl Into<String>,
        author_email: impl Into<String>,
    ) -> Self {
        Self {
            repo_path: repo_path.into(),
            author_name: author_name.into(),
            author_email: author_email.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<RebaseFinishOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let name = self.author_name.clone();
        let email = self.author_email.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let mut rebase = repo.open_rebase(None)?;
            let sig = Signature::now(&name, &email)?;
            rebase.finish(Some(&sig))?;
            Ok(RebaseFinishOutput { finished: true })
        })
        .await
    }
}

#[async_trait]
impl Operation for RebaseFinish {
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

impl TypedOperation for RebaseFinish {
    type Output = RebaseFinishOutput;
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use git2::{Repository, Signature};
    use ironflow_core::operation::Operation;

    use super::*;
    use crate::test_helpers::ctx;

    fn init_with_branch(path: &Path) {
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
        tb.insert("main.txt", repo.blob(b"main-only").unwrap(), 0o100644)
            .unwrap();
        let main_tree = repo.find_tree(tb.write().unwrap()).unwrap();
        repo.commit(
            Some("HEAD"),
            &sig,
            &sig,
            "main commit",
            &main_tree,
            &[&base],
        )
        .unwrap();

        let mut tb2 = repo.treebuilder(Some(&tree)).unwrap();
        tb2.insert("feat.txt", repo.blob(b"feat-only").unwrap(), 0o100644)
            .unwrap();
        let feat_tree = repo.find_tree(tb2.write().unwrap()).unwrap();
        repo.commit(
            Some("refs/heads/feature"),
            &sig,
            &sig,
            "feat commit",
            &feat_tree,
            &[&base],
        )
        .unwrap();

        repo.checkout_head(Some(CheckoutBuilder::new().force()))
            .unwrap();
    }

    #[tokio::test]
    async fn rebase_init_counts_operations() {
        let tmp = tempfile::tempdir().unwrap();
        init_with_branch(tmp.path());
        let result = RebaseInit::new(tmp.path(), "feature", "master")
            .run(&ctx())
            .await
            .unwrap();
        assert_eq!(result.branch, "feature");
        assert!(result.operations > 0);
    }

    #[tokio::test]
    async fn rebase_init_and_abort() {
        let tmp = tempfile::tempdir().unwrap();
        init_with_branch(tmp.path());
        RebaseInit::new(tmp.path(), "feature", "master")
            .run(&ctx())
            .await
            .unwrap();
        let result = RebaseAbort::new(tmp.path()).run(&ctx()).await.unwrap();
        assert!(result.aborted);
    }

    #[tokio::test]
    async fn execute_serializes_correctly() {
        let tmp = tempfile::tempdir().unwrap();
        init_with_branch(tmp.path());
        let value = RebaseInit::new(tmp.path(), "feature", "master")
            .execute(&ctx())
            .await
            .unwrap();
        assert_eq!(value["branch"], "feature");
        assert!(value["operations"].is_number());
    }
}
