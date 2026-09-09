//! Log / history operations.

use std::path::PathBuf;

use async_trait::async_trait;
use git2::{Repository, Sort};
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::helpers::{blocking, to_value};

/// A commit entry in a revwalk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogCommitEntry {
    pub oid: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time: Option<i64>,
}

/// Output of revwalk operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevwalkOutput {
    pub commits: Vec<LogCommitEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<String>,
}

/// Create a new revision walk from HEAD.
///
/// Returns the first `limit` commits reachable from HEAD.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::log::RevwalkNew;
/// use ironflow_core::operation::Operation;
///
/// let op = RevwalkNew::new("/path/to/repo", 50);
/// assert_eq!(op.kind(), "git");
/// ```
pub struct RevwalkNew {
    repo_path: PathBuf,
    limit: usize,
}

impl RevwalkNew {
    /// Create a new revwalk operation.
    pub fn new(repo_path: impl Into<PathBuf>, limit: usize) -> Self {
        Self {
            repo_path: repo_path.into(),
            limit,
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<RevwalkOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let limit = self.limit;
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let mut revwalk = repo.revwalk()?;
            revwalk.push_head()?;
            revwalk.set_sorting(Sort::TIME)?;
            let commits = revwalk
                .take(limit)
                .filter_map(|oid| oid.ok())
                .filter_map(|oid| repo.find_commit(oid).ok())
                .map(|c| LogCommitEntry {
                    oid: c.id().to_string(),
                    message: c.message().unwrap_or("").to_string(),
                    author: Some(c.author().name().unwrap_or("").to_string()),
                    time: Some(c.time().seconds()),
                })
                .collect();
            Ok(RevwalkOutput {
                commits,
                range: None,
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for RevwalkNew {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "repo_path": self.repo_path, "limit": self.limit }))
    }
}

impl TypedOperation for RevwalkNew {
    type Output = RevwalkOutput;
}

/// Walk commits in a range (from..to).
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::log::RevwalkPushRange;
/// use ironflow_core::operation::Operation;
///
/// let op = RevwalkPushRange::new("/path/to/repo", "abc123..def456", 100);
/// assert_eq!(op.kind(), "git");
/// ```
pub struct RevwalkPushRange {
    repo_path: PathBuf,
    range: String,
    limit: usize,
}

impl RevwalkPushRange {
    /// Create a new range-walk operation.
    pub fn new(repo_path: impl Into<PathBuf>, range: impl Into<String>, limit: usize) -> Self {
        Self {
            repo_path: repo_path.into(),
            range: range.into(),
            limit,
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<RevwalkOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let range = self.range.clone();
        let limit = self.limit;
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let mut revwalk = repo.revwalk()?;
            revwalk.push_range(&range)?;
            let commits = revwalk
                .take(limit)
                .filter_map(|oid| oid.ok())
                .filter_map(|oid| repo.find_commit(oid).ok())
                .map(|c| LogCommitEntry {
                    oid: c.id().to_string(),
                    message: c.message().unwrap_or("").to_string(),
                    author: Some(c.author().name().unwrap_or("").to_string()),
                    time: Some(c.time().seconds()),
                })
                .collect();
            Ok(RevwalkOutput {
                commits,
                range: Some(range),
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for RevwalkPushRange {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "repo_path": self.repo_path, "range": self.range }))
    }
}

impl TypedOperation for RevwalkPushRange {
    type Output = RevwalkOutput;
}

/// Walk commits following only first parents (no merge parents).
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::log::RevwalkSimplifyFirstParent;
/// use ironflow_core::operation::Operation;
///
/// let op = RevwalkSimplifyFirstParent::new("/path/to/repo", 50);
/// assert_eq!(op.kind(), "git");
/// ```
pub struct RevwalkSimplifyFirstParent {
    repo_path: PathBuf,
    limit: usize,
}

impl RevwalkSimplifyFirstParent {
    /// Create a new first-parent walk operation.
    pub fn new(repo_path: impl Into<PathBuf>, limit: usize) -> Self {
        Self {
            repo_path: repo_path.into(),
            limit,
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<RevwalkOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let limit = self.limit;
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let mut revwalk = repo.revwalk()?;
            revwalk.push_head()?;
            revwalk.simplify_first_parent()?;
            let commits = revwalk
                .take(limit)
                .filter_map(|oid| oid.ok())
                .filter_map(|oid| repo.find_commit(oid).ok())
                .map(|c| LogCommitEntry {
                    oid: c.id().to_string(),
                    message: c.message().unwrap_or("").to_string(),
                    author: Some(c.author().name().unwrap_or("").to_string()),
                    time: Some(c.time().seconds()),
                })
                .collect();
            Ok(RevwalkOutput {
                commits,
                range: None,
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for RevwalkSimplifyFirstParent {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "repo_path": self.repo_path, "limit": self.limit }))
    }
}

impl TypedOperation for RevwalkSimplifyFirstParent {
    type Output = RevwalkOutput;
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use git2::{Commit, Repository, Signature};
    use ironflow_core::operation::Operation;

    use super::*;
    use crate::test_helpers::ctx;

    fn init_repo_n_commits(path: &Path, n: usize) {
        let repo = Repository::init(path).unwrap();
        let sig = Signature::now("Test", "test@test.com").unwrap();
        let mut parent = None;
        for i in 0..n {
            fs::write(path.join("file.txt"), format!("v{i}")).unwrap();
            let mut index = repo.index().unwrap();
            index.add_path(Path::new("file.txt")).unwrap();
            index.write().unwrap();
            let oid = index.write_tree().unwrap();
            let tree = repo.find_tree(oid).unwrap();
            let parents: Vec<&Commit<'_>> = parent.iter().collect();
            let c = repo
                .commit(
                    Some("HEAD"),
                    &sig,
                    &sig,
                    &format!("commit {i}"),
                    &tree,
                    &parents,
                )
                .unwrap();
            parent = Some(repo.find_commit(c).unwrap());
        }
    }

    #[tokio::test]
    async fn revwalk_returns_commits() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo_n_commits(tmp.path(), 3);
        let result = RevwalkNew::new(tmp.path(), 10).run(&ctx()).await.unwrap();
        assert_eq!(result.commits.len(), 3);
        assert_eq!(result.commits[0].message, "commit 2");
        assert!(result.commits[0].author.is_some());
    }

    #[tokio::test]
    async fn revwalk_respects_limit() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo_n_commits(tmp.path(), 5);
        let result = RevwalkNew::new(tmp.path(), 2).run(&ctx()).await.unwrap();
        assert_eq!(result.commits.len(), 2);
    }

    #[tokio::test]
    async fn revwalk_push_range() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo_n_commits(tmp.path(), 3);
        let repo = Repository::open(tmp.path()).unwrap();
        let mut revwalk = repo.revwalk().unwrap();
        revwalk.push_head().unwrap();
        let oids: Vec<_> = revwalk.filter_map(|o| o.ok()).collect();
        let range = format!("{}..{}", oids[2], oids[0]);
        let result = RevwalkPushRange::new(tmp.path(), &range, 100)
            .run(&ctx())
            .await
            .unwrap();
        assert_eq!(result.commits.len(), 2);
        assert_eq!(result.range.as_deref(), Some(range.as_str()));
    }

    #[tokio::test]
    async fn simplify_first_parent() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo_n_commits(tmp.path(), 3);
        let result = RevwalkSimplifyFirstParent::new(tmp.path(), 10)
            .run(&ctx())
            .await
            .unwrap();
        assert_eq!(result.commits.len(), 3);
    }

    #[tokio::test]
    async fn execute_serializes_correctly() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo_n_commits(tmp.path(), 2);
        let value = RevwalkNew::new(tmp.path(), 10)
            .execute(&ctx())
            .await
            .unwrap();
        assert_eq!(value["commits"].as_array().unwrap().len(), 2);
    }
}
