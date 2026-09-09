//! Checkout operations.

use std::path::PathBuf;

use async_trait::async_trait;
use git2::build::CheckoutBuilder;
use git2::{Oid, Repository};
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::helpers::{blocking, to_value};

/// Output of checkout operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckoutOutput {
    pub checked_out: String,
}

/// Checkout HEAD (reset the working directory to HEAD).
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::checkout::CheckoutHead;
/// use ironflow_core::operation::Operation;
///
/// let op = CheckoutHead::new("/path/to/repo");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct CheckoutHead {
    repo_path: PathBuf,
}

impl CheckoutHead {
    /// Create a new checkout-head operation.
    pub fn new(repo_path: impl Into<PathBuf>) -> Self {
        Self {
            repo_path: repo_path.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<CheckoutOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            repo.checkout_head(Some(CheckoutBuilder::new().force()))?;
            Ok(CheckoutOutput {
                checked_out: "HEAD".to_string(),
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for CheckoutHead {
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

impl TypedOperation for CheckoutHead {
    type Output = CheckoutOutput;
}

/// Checkout the index (update the working directory from the index).
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::checkout::CheckoutIndex;
/// use ironflow_core::operation::Operation;
///
/// let op = CheckoutIndex::new("/path/to/repo");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct CheckoutIndex {
    repo_path: PathBuf,
}

impl CheckoutIndex {
    /// Create a new checkout-index operation.
    pub fn new(repo_path: impl Into<PathBuf>) -> Self {
        Self {
            repo_path: repo_path.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<CheckoutOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            repo.checkout_index(None, Some(CheckoutBuilder::new().force()))?;
            Ok(CheckoutOutput {
                checked_out: "index".to_string(),
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for CheckoutIndex {
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

impl TypedOperation for CheckoutIndex {
    type Output = CheckoutOutput;
}

/// Checkout a specific tree object.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::checkout::CheckoutTree;
/// use ironflow_core::operation::Operation;
///
/// let op = CheckoutTree::new("/path/to/repo", "abc123");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct CheckoutTree {
    repo_path: PathBuf,
    treeish: String,
}

impl CheckoutTree {
    /// Create a new checkout-tree operation.
    pub fn new(repo_path: impl Into<PathBuf>, treeish: impl Into<String>) -> Self {
        Self {
            repo_path: repo_path.into(),
            treeish: treeish.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<CheckoutOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let treeish = self.treeish.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let oid = Oid::from_str(&treeish)?;
            let object = repo.find_object(oid, None)?;
            repo.checkout_tree(&object, Some(CheckoutBuilder::new().force()))?;
            Ok(CheckoutOutput {
                checked_out: treeish,
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for CheckoutTree {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "repo_path": self.repo_path, "treeish": self.treeish }))
    }
}

impl TypedOperation for CheckoutTree {
    type Output = CheckoutOutput;
}

#[cfg(test)]
mod tests {
    use std::fs;

    use git2::Repository;
    use ironflow_core::operation::Operation;

    use super::*;
    use crate::test_helpers::{ctx, init_repo};

    #[tokio::test]
    async fn checkout_head_restores_workdir() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        fs::write(tmp.path().join("file.txt"), "dirty").unwrap();
        let result = CheckoutHead::new(tmp.path()).run(&ctx()).await.unwrap();
        assert_eq!(result.checked_out, "HEAD");
        assert_eq!(
            fs::read_to_string(tmp.path().join("file.txt")).unwrap(),
            "content"
        );
    }

    #[tokio::test]
    async fn checkout_index_restores_from_index() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        fs::write(tmp.path().join("file.txt"), "dirty").unwrap();
        let result = CheckoutIndex::new(tmp.path()).run(&ctx()).await.unwrap();
        assert_eq!(result.checked_out, "index");
        assert_eq!(
            fs::read_to_string(tmp.path().join("file.txt")).unwrap(),
            "content"
        );
    }

    #[tokio::test]
    async fn checkout_tree_with_commit_oid() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        let repo = Repository::open(tmp.path()).unwrap();
        let oid = repo
            .head()
            .unwrap()
            .peel_to_commit()
            .unwrap()
            .id()
            .to_string();
        let result = CheckoutTree::new(tmp.path(), &oid)
            .run(&ctx())
            .await
            .unwrap();
        assert_eq!(result.checked_out, oid);
    }

    #[tokio::test]
    async fn checkout_tree_invalid_oid_fails() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        assert!(
            CheckoutTree::new(tmp.path(), "bad")
                .run(&ctx())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn execute_serializes_correctly() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        let value = CheckoutHead::new(tmp.path()).execute(&ctx()).await.unwrap();
        assert_eq!(value["checked_out"], "HEAD");
    }
}
