//! Cherry-pick and revert operations.

use std::path::PathBuf;

use async_trait::async_trait;
use git2::{Oid, Repository};
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::helpers::{blocking, to_value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CherrypickOutput {
    pub oid: String,
    pub applied: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CherrypickConflictOutput {
    pub has_conflicts: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevertOutput {
    pub oid: String,
    pub reverted: bool,
}

/// Cherry-pick a commit onto the working directory and index.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::cherrypick::Cherrypick;
/// use ironflow_core::operation::Operation;
///
/// let op = Cherrypick::new("/path/to/repo", "abc123");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct Cherrypick {
    repo_path: PathBuf,
    oid: String,
}

impl Cherrypick {
    /// Create a new cherry-pick operation.
    pub fn new(repo_path: impl Into<PathBuf>, oid: impl Into<String>) -> Self {
        Self {
            repo_path: repo_path.into(),
            oid: oid.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<CherrypickOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let oid_str = self.oid.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let oid = Oid::from_str(&oid_str)?;
            let commit = repo.find_commit(oid)?;
            repo.cherrypick(&commit, None)?;
            Ok(CherrypickOutput {
                oid: oid_str,
                applied: true,
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for Cherrypick {
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

impl TypedOperation for Cherrypick {
    type Output = CherrypickOutput;
}

/// Cherry-pick a commit as a tree merge (without touching the working directory).
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::cherrypick::CherrypickCommit;
/// use ironflow_core::operation::Operation;
///
/// let op = CherrypickCommit::new("/path/to/repo", "abc123", "def456");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct CherrypickCommit {
    repo_path: PathBuf,
    cherrypick_oid: String,
    our_oid: String,
}

impl CherrypickCommit {
    /// Create a new cherry-pick-commit operation.
    pub fn new(
        repo_path: impl Into<PathBuf>,
        cherrypick_oid: impl Into<String>,
        our_oid: impl Into<String>,
    ) -> Self {
        Self {
            repo_path: repo_path.into(),
            cherrypick_oid: cherrypick_oid.into(),
            our_oid: our_oid.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(
        &self,
        _ctx: &OperationContext,
    ) -> Result<CherrypickConflictOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let cp_oid = self.cherrypick_oid.clone();
        let our_oid = self.our_oid.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let cp_commit = repo.find_commit(Oid::from_str(&cp_oid)?)?;
            let our_commit = repo.find_commit(Oid::from_str(&our_oid)?)?;
            let index = repo.cherrypick_commit(&cp_commit, &our_commit, 0, None)?;
            Ok(CherrypickConflictOutput {
                has_conflicts: index.has_conflicts(),
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for CherrypickCommit {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(
            serde_json::json!({ "repo_path": self.repo_path, "cherrypick": self.cherrypick_oid, "our": self.our_oid }),
        )
    }
}

impl TypedOperation for CherrypickCommit {
    type Output = CherrypickConflictOutput;
}

/// Revert a commit onto the working directory and index.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::cherrypick::Revert;
/// use ironflow_core::operation::Operation;
///
/// let op = Revert::new("/path/to/repo", "abc123");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct Revert {
    repo_path: PathBuf,
    oid: String,
}

impl Revert {
    /// Create a new revert operation.
    pub fn new(repo_path: impl Into<PathBuf>, oid: impl Into<String>) -> Self {
        Self {
            repo_path: repo_path.into(),
            oid: oid.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<RevertOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let oid_str = self.oid.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let oid = Oid::from_str(&oid_str)?;
            let commit = repo.find_commit(oid)?;
            repo.revert(&commit, None)?;
            Ok(RevertOutput {
                oid: oid_str,
                reverted: true,
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for Revert {
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

impl TypedOperation for Revert {
    type Output = RevertOutput;
}

/// Revert a commit as a tree merge (without touching the working directory).
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::cherrypick::RevertCommit;
/// use ironflow_core::operation::Operation;
///
/// let op = RevertCommit::new("/path/to/repo", "abc123", "def456");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct RevertCommit {
    repo_path: PathBuf,
    revert_oid: String,
    our_oid: String,
}

impl RevertCommit {
    /// Create a new revert-commit operation.
    pub fn new(
        repo_path: impl Into<PathBuf>,
        revert_oid: impl Into<String>,
        our_oid: impl Into<String>,
    ) -> Self {
        Self {
            repo_path: repo_path.into(),
            revert_oid: revert_oid.into(),
            our_oid: our_oid.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(
        &self,
        _ctx: &OperationContext,
    ) -> Result<CherrypickConflictOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let rv_oid = self.revert_oid.clone();
        let our_oid = self.our_oid.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let rv_commit = repo.find_commit(Oid::from_str(&rv_oid)?)?;
            let our_commit = repo.find_commit(Oid::from_str(&our_oid)?)?;
            let index = repo.revert_commit(&rv_commit, &our_commit, 0, None)?;
            Ok(CherrypickConflictOutput {
                has_conflicts: index.has_conflicts(),
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for RevertCommit {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(
            serde_json::json!({ "repo_path": self.repo_path, "revert": self.revert_oid, "our": self.our_oid }),
        )
    }
}

impl TypedOperation for RevertCommit {
    type Output = CherrypickConflictOutput;
}

#[cfg(test)]
mod tests {
    use ironflow_core::operation::Operation;

    use super::*;
    use crate::test_helpers::{ctx, make_two_commits};

    #[tokio::test]
    async fn cherrypick_commit_no_conflict() {
        let tmp = tempfile::tempdir().unwrap();
        let (c1, c2) = make_two_commits(tmp.path());
        let result = CherrypickCommit::new(tmp.path(), &c2, &c1)
            .run(&ctx())
            .await
            .unwrap();
        assert!(!result.has_conflicts);
    }

    #[tokio::test]
    async fn revert_commit_no_conflict() {
        let tmp = tempfile::tempdir().unwrap();
        let (_c1, c2) = make_two_commits(tmp.path());
        let result = RevertCommit::new(tmp.path(), &c2, &c2)
            .run(&ctx())
            .await
            .unwrap();
        assert!(!result.has_conflicts);
    }

    #[tokio::test]
    async fn cherrypick_applies_to_workdir() {
        let tmp = tempfile::tempdir().unwrap();
        let (_, c2) = make_two_commits(tmp.path());
        let result = Cherrypick::new(tmp.path(), &c2).run(&ctx()).await.unwrap();
        assert!(result.applied);
        assert_eq!(result.oid, c2);
    }

    #[tokio::test]
    async fn cherrypick_invalid_oid_fails() {
        let tmp = tempfile::tempdir().unwrap();
        make_two_commits(tmp.path());
        assert!(
            Cherrypick::new(tmp.path(), "bad-oid")
                .run(&ctx())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn execute_serializes_correctly() {
        let tmp = tempfile::tempdir().unwrap();
        let (c1, c2) = make_two_commits(tmp.path());
        let value = CherrypickCommit::new(tmp.path(), &c2, &c1)
            .execute(&ctx())
            .await
            .unwrap();
        assert!(!value["has_conflicts"].as_bool().unwrap());
    }
}
