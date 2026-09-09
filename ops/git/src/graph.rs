//! Graph operations (ahead/behind, descendant, describe).

use std::path::PathBuf;

use async_trait::async_trait;
use git2::{DescribeFormatOptions, DescribeOptions, Oid, Repository};
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::helpers::{blocking, to_value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphAheadBehindOutput {
    pub ahead: usize,
    pub behind: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphDescendantOfOutput {
    pub is_descendant: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphDescribeOutput {
    pub description: String,
}

/// Count commits ahead and behind between two references.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::graph::GraphAheadBehind;
/// use ironflow_core::operation::Operation;
///
/// let op = GraphAheadBehind::new("/path/to/repo", "abc123", "def456");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct GraphAheadBehind {
    repo_path: PathBuf,
    local: String,
    upstream: String,
}

impl GraphAheadBehind {
    /// Create a new ahead-behind operation.
    pub fn new(
        repo_path: impl Into<PathBuf>,
        local: impl Into<String>,
        upstream: impl Into<String>,
    ) -> Self {
        Self {
            repo_path: repo_path.into(),
            local: local.into(),
            upstream: upstream.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(
        &self,
        _ctx: &OperationContext,
    ) -> Result<GraphAheadBehindOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let local = self.local.clone();
        let upstream = self.upstream.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let local_oid = Oid::from_str(&local)?;
            let upstream_oid = Oid::from_str(&upstream)?;
            let (ahead, behind) = repo.graph_ahead_behind(local_oid, upstream_oid)?;
            Ok(GraphAheadBehindOutput { ahead, behind })
        })
        .await
    }
}

#[async_trait]
impl Operation for GraphAheadBehind {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(
            serde_json::json!({ "repo_path": self.repo_path, "local": self.local, "upstream": self.upstream }),
        )
    }
}

impl TypedOperation for GraphAheadBehind {
    type Output = GraphAheadBehindOutput;
}

/// Check if a commit is a descendant of another.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::graph::GraphDescendantOf;
/// use ironflow_core::operation::Operation;
///
/// let op = GraphDescendantOf::new("/path/to/repo", "abc123", "def456");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct GraphDescendantOf {
    repo_path: PathBuf,
    commit: String,
    ancestor: String,
}

impl GraphDescendantOf {
    /// Create a new descendant-of check operation.
    pub fn new(
        repo_path: impl Into<PathBuf>,
        commit: impl Into<String>,
        ancestor: impl Into<String>,
    ) -> Self {
        Self {
            repo_path: repo_path.into(),
            commit: commit.into(),
            ancestor: ancestor.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(
        &self,
        _ctx: &OperationContext,
    ) -> Result<GraphDescendantOfOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let commit = self.commit.clone();
        let ancestor = self.ancestor.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let commit_oid = Oid::from_str(&commit)?;
            let ancestor_oid = Oid::from_str(&ancestor)?;
            let is_descendant = repo.graph_descendant_of(commit_oid, ancestor_oid)?;
            Ok(GraphDescendantOfOutput { is_descendant })
        })
        .await
    }
}

#[async_trait]
impl Operation for GraphDescendantOf {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(
            serde_json::json!({ "repo_path": self.repo_path, "commit": self.commit, "ancestor": self.ancestor }),
        )
    }
}

impl TypedOperation for GraphDescendantOf {
    type Output = GraphDescendantOfOutput;
}

/// Describe a commit using the most recent tag reachable from it.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::graph::GraphDescribe;
/// use ironflow_core::operation::Operation;
///
/// let op = GraphDescribe::new("/path/to/repo");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct GraphDescribe {
    repo_path: PathBuf,
}

impl GraphDescribe {
    /// Create a new describe operation.
    pub fn new(repo_path: impl Into<PathBuf>) -> Self {
        Self {
            repo_path: repo_path.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(
        &self,
        _ctx: &OperationContext,
    ) -> Result<GraphDescribeOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let describe = repo.describe(DescribeOptions::new().describe_tags())?;
            let formatted =
                describe.format(Some(DescribeFormatOptions::new().dirty_suffix("-dirty")))?;
            Ok(GraphDescribeOutput {
                description: formatted,
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for GraphDescribe {
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

impl TypedOperation for GraphDescribe {
    type Output = GraphDescribeOutput;
}

#[cfg(test)]
mod tests {
    use git2::Repository;
    use ironflow_core::operation::Operation;

    use super::*;
    use crate::test_helpers::{ctx, make_two_commits};

    #[tokio::test]
    async fn ahead_behind_counts() {
        let tmp = tempfile::tempdir().unwrap();
        let (c1, c2) = make_two_commits(tmp.path());
        let result = GraphAheadBehind::new(tmp.path(), &c2, &c1)
            .run(&ctx())
            .await
            .unwrap();
        assert_eq!(result.ahead, 1);
        assert_eq!(result.behind, 0);

        let result = GraphAheadBehind::new(tmp.path(), &c1, &c2)
            .run(&ctx())
            .await
            .unwrap();
        assert_eq!(result.ahead, 0);
        assert_eq!(result.behind, 1);
    }

    #[tokio::test]
    async fn descendant_of() {
        let tmp = tempfile::tempdir().unwrap();
        let (c1, c2) = make_two_commits(tmp.path());
        let result = GraphDescendantOf::new(tmp.path(), &c2, &c1)
            .run(&ctx())
            .await
            .unwrap();
        assert!(result.is_descendant);
        let result = GraphDescendantOf::new(tmp.path(), &c1, &c2)
            .run(&ctx())
            .await
            .unwrap();
        assert!(!result.is_descendant);
    }

    #[tokio::test]
    async fn describe_with_tag() {
        let tmp = tempfile::tempdir().unwrap();
        make_two_commits(tmp.path());
        let repo = Repository::open(tmp.path()).unwrap();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.tag_lightweight("v1.0", head.as_object(), false)
            .unwrap();
        let result = GraphDescribe::new(tmp.path()).run(&ctx()).await.unwrap();
        assert!(result.description.contains("v1.0"));
    }

    #[tokio::test]
    async fn execute_serializes_correctly() {
        let tmp = tempfile::tempdir().unwrap();
        let (c1, c2) = make_two_commits(tmp.path());
        let value = GraphAheadBehind::new(tmp.path(), &c2, &c1)
            .execute(&ctx())
            .await
            .unwrap();
        assert_eq!(value["ahead"], 1);
    }
}
