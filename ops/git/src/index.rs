//! Index / staging area operations.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use git2::{IndexAddOption, Repository};
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::helpers::{blocking, to_value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexPathOutput {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexPathspecsOutput {
    pub pathspecs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexUpdateAllOutput {
    pub updated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexWriteTreeOutput {
    pub tree_oid: String,
}

/// Add a single file to the index.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::index::IndexAdd;
/// use ironflow_core::operation::Operation;
///
/// let op = IndexAdd::new("/path/to/repo", "file.txt");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct IndexAdd {
    repo_path: PathBuf,
    path: String,
}

impl IndexAdd {
    /// Create a new add operation.
    pub fn new(repo_path: impl Into<PathBuf>, path: impl Into<String>) -> Self {
        Self {
            repo_path: repo_path.into(),
            path: path.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<IndexPathOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let file_path = self.path.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let mut index = repo.index()?;
            index.add_path(Path::new(&file_path))?;
            index.write()?;
            Ok(IndexPathOutput { path: file_path })
        })
        .await
    }
}

#[async_trait]
impl Operation for IndexAdd {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "repo_path": self.repo_path, "path": self.path }))
    }
}

impl TypedOperation for IndexAdd {
    type Output = IndexPathOutput;
}

/// Add all files matching a pathspec to the index.
///
/// Equivalent to `git add .` when called with `["*"]` or `["."]`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::index::IndexAddAll;
/// use ironflow_core::operation::Operation;
///
/// let op = IndexAddAll::new("/path/to/repo", vec!["*.rs"]);
/// assert_eq!(op.kind(), "git");
/// ```
pub struct IndexAddAll {
    repo_path: PathBuf,
    pathspecs: Vec<String>,
}

impl IndexAddAll {
    /// Create a new add-all operation.
    pub fn new(repo_path: impl Into<PathBuf>, pathspecs: Vec<impl Into<String>>) -> Self {
        Self {
            repo_path: repo_path.into(),
            pathspecs: pathspecs.into_iter().map(Into::into).collect(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(
        &self,
        _ctx: &OperationContext,
    ) -> Result<IndexPathspecsOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let pathspecs = self.pathspecs.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let mut index = repo.index()?;
            index.add_all(&pathspecs, IndexAddOption::DEFAULT, None)?;
            index.write()?;
            Ok(IndexPathspecsOutput { pathspecs })
        })
        .await
    }
}

#[async_trait]
impl Operation for IndexAddAll {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "repo_path": self.repo_path, "pathspecs": self.pathspecs }))
    }
}

impl TypedOperation for IndexAddAll {
    type Output = IndexPathspecsOutput;
}

/// Remove a file from the index.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::index::IndexRemove;
/// use ironflow_core::operation::Operation;
///
/// let op = IndexRemove::new("/path/to/repo", "file.txt");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct IndexRemove {
    repo_path: PathBuf,
    path: String,
}

impl IndexRemove {
    /// Create a new remove operation.
    pub fn new(repo_path: impl Into<PathBuf>, path: impl Into<String>) -> Self {
        Self {
            repo_path: repo_path.into(),
            path: path.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<IndexPathOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let file_path = self.path.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let mut index = repo.index()?;
            index.remove_path(Path::new(&file_path))?;
            index.write()?;
            Ok(IndexPathOutput { path: file_path })
        })
        .await
    }
}

#[async_trait]
impl Operation for IndexRemove {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "repo_path": self.repo_path, "path": self.path }))
    }
}

impl TypedOperation for IndexRemove {
    type Output = IndexPathOutput;
}

/// Remove all files matching a pathspec from the index.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::index::IndexRemoveAll;
/// use ironflow_core::operation::Operation;
///
/// let op = IndexRemoveAll::new("/path/to/repo", vec!["*.tmp"]);
/// assert_eq!(op.kind(), "git");
/// ```
pub struct IndexRemoveAll {
    repo_path: PathBuf,
    pathspecs: Vec<String>,
}

impl IndexRemoveAll {
    /// Create a new remove-all operation.
    pub fn new(repo_path: impl Into<PathBuf>, pathspecs: Vec<impl Into<String>>) -> Self {
        Self {
            repo_path: repo_path.into(),
            pathspecs: pathspecs.into_iter().map(Into::into).collect(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(
        &self,
        _ctx: &OperationContext,
    ) -> Result<IndexPathspecsOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let pathspecs = self.pathspecs.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let mut index = repo.index()?;
            index.remove_all(&pathspecs, None)?;
            index.write()?;
            Ok(IndexPathspecsOutput { pathspecs })
        })
        .await
    }
}

#[async_trait]
impl Operation for IndexRemoveAll {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "repo_path": self.repo_path, "pathspecs": self.pathspecs }))
    }
}

impl TypedOperation for IndexRemoveAll {
    type Output = IndexPathspecsOutput;
}

/// Update all tracked files in the index.
///
/// Updates the index with the current content of tracked files.
/// Equivalent to `git add -u`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::index::IndexUpdateAll;
/// use ironflow_core::operation::Operation;
///
/// let op = IndexUpdateAll::new("/path/to/repo");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct IndexUpdateAll {
    repo_path: PathBuf,
}

impl IndexUpdateAll {
    /// Create a new update-all operation.
    pub fn new(repo_path: impl Into<PathBuf>) -> Self {
        Self {
            repo_path: repo_path.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(
        &self,
        _ctx: &OperationContext,
    ) -> Result<IndexUpdateAllOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let mut index = repo.index()?;
            index.update_all(["*"], None)?;
            index.write()?;
            Ok(IndexUpdateAllOutput { updated: true })
        })
        .await
    }
}

#[async_trait]
impl Operation for IndexUpdateAll {
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

impl TypedOperation for IndexUpdateAll {
    type Output = IndexUpdateAllOutput;
}

/// Write the index as a tree object.
///
/// Converts the current index into a tree object in the object database.
/// Returns the tree OID.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::index::IndexWriteTree;
/// use ironflow_core::operation::Operation;
///
/// let op = IndexWriteTree::new("/path/to/repo");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct IndexWriteTree {
    repo_path: PathBuf,
}

impl IndexWriteTree {
    /// Create a new write-tree operation.
    pub fn new(repo_path: impl Into<PathBuf>) -> Self {
        Self {
            repo_path: repo_path.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(
        &self,
        _ctx: &OperationContext,
    ) -> Result<IndexWriteTreeOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let mut index = repo.index()?;
            let oid = index.write_tree()?;
            Ok(IndexWriteTreeOutput {
                tree_oid: oid.to_string(),
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for IndexWriteTree {
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

impl TypedOperation for IndexWriteTree {
    type Output = IndexWriteTreeOutput;
}

#[cfg(test)]
mod tests {
    use std::fs;

    use ironflow_core::operation::Operation;

    use super::*;
    use crate::test_helpers::{ctx, init_repo};

    #[tokio::test]
    async fn add_and_remove() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        fs::write(tmp.path().join("new.txt"), "n").unwrap();
        let result = IndexAdd::new(tmp.path(), "new.txt")
            .run(&ctx())
            .await
            .unwrap();
        assert_eq!(result.path, "new.txt");
        let result = IndexRemove::new(tmp.path(), "new.txt")
            .run(&ctx())
            .await
            .unwrap();
        assert_eq!(result.path, "new.txt");
    }

    #[tokio::test]
    async fn add_all_with_glob() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        fs::write(tmp.path().join("a.rs"), "a").unwrap();
        fs::write(tmp.path().join("b.rs"), "b").unwrap();
        let result = IndexAddAll::new(tmp.path(), vec!["*.rs"])
            .run(&ctx())
            .await
            .unwrap();
        assert_eq!(result.pathspecs, vec!["*.rs"]);
    }

    #[tokio::test]
    async fn update_all() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        fs::write(tmp.path().join("file.txt"), "modified").unwrap();
        let result = IndexUpdateAll::new(tmp.path()).run(&ctx()).await.unwrap();
        assert!(result.updated);
    }

    #[tokio::test]
    async fn write_tree_returns_oid() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        let result = IndexWriteTree::new(tmp.path()).run(&ctx()).await.unwrap();
        assert!(!result.tree_oid.is_empty());
    }

    #[tokio::test]
    async fn add_nonexistent_file_fails() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        assert!(
            IndexAdd::new(tmp.path(), "nope.txt")
                .run(&ctx())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn execute_serializes_correctly() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        let value = IndexWriteTree::new(tmp.path())
            .execute(&ctx())
            .await
            .unwrap();
        assert!(value["tree_oid"].is_string());
    }
}
