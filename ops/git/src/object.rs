//! Object operations (blob, tree, find).

use std::path::PathBuf;

use async_trait::async_trait;
use git2::{ObjectType, Oid, Repository};
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::helpers::{blocking, to_value};

fn object_type_label(kind: Option<ObjectType>) -> &'static str {
    match kind {
        Some(ObjectType::Commit) => "commit",
        Some(ObjectType::Tree) => "tree",
        Some(ObjectType::Blob) => "blob",
        Some(ObjectType::Tag) => "tag",
        _ => "unknown",
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlobCreateOutput {
    pub oid: String,
    pub size: usize,
}

/// A tree entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreeEntryOutput {
    pub name: String,
    pub oid: String,
    pub kind: String,
    pub filemode: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreeLookupOutput {
    pub oid: String,
    pub entries: Vec<TreeEntryOutput>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FindObjectOutput {
    pub oid: String,
    #[serde(rename = "type")]
    pub object_type: String,
}

/// Create a blob from content.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::object::BlobCreate;
/// use ironflow_core::operation::Operation;
///
/// let op = BlobCreate::new("/path/to/repo", b"hello world".to_vec());
/// assert_eq!(op.kind(), "git");
/// ```
pub struct BlobCreate {
    repo_path: PathBuf,
    content: Vec<u8>,
}

impl BlobCreate {
    /// Create a new blob-create operation.
    pub fn new(repo_path: impl Into<PathBuf>, content: Vec<u8>) -> Self {
        Self {
            repo_path: repo_path.into(),
            content,
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<BlobCreateOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let content = self.content.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let oid = repo.blob(&content)?;
            Ok(BlobCreateOutput {
                oid: oid.to_string(),
                size: content.len(),
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for BlobCreate {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "repo_path": self.repo_path, "size": self.content.len() }))
    }
}

impl TypedOperation for BlobCreate {
    type Output = BlobCreateOutput;
}

/// Look up a tree by OID and list its entries.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::object::TreeLookup;
/// use ironflow_core::operation::Operation;
///
/// let op = TreeLookup::new("/path/to/repo", "abc123");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct TreeLookup {
    repo_path: PathBuf,
    oid: String,
}

impl TreeLookup {
    /// Create a new tree-lookup operation.
    pub fn new(repo_path: impl Into<PathBuf>, oid: impl Into<String>) -> Self {
        Self {
            repo_path: repo_path.into(),
            oid: oid.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<TreeLookupOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let oid_str = self.oid.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let oid = Oid::from_str(&oid_str)?;
            let tree = repo.find_tree(oid)?;
            let entries = tree
                .iter()
                .map(|entry| TreeEntryOutput {
                    name: entry.name().unwrap_or("").to_string(),
                    oid: entry.id().to_string(),
                    kind: object_type_label(entry.kind()).to_string(),
                    filemode: entry.filemode(),
                })
                .collect();
            Ok(TreeLookupOutput {
                oid: oid_str,
                entries,
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for TreeLookup {
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

impl TypedOperation for TreeLookup {
    type Output = TreeLookupOutput;
}

/// Find an object by OID.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::object::FindObject;
/// use ironflow_core::operation::Operation;
///
/// let op = FindObject::new("/path/to/repo", "abc123");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct FindObject {
    repo_path: PathBuf,
    oid: String,
}

impl FindObject {
    /// Create a new find-object operation.
    pub fn new(repo_path: impl Into<PathBuf>, oid: impl Into<String>) -> Self {
        Self {
            repo_path: repo_path.into(),
            oid: oid.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<FindObjectOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let oid_str = self.oid.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let oid = Oid::from_str(&oid_str)?;
            let obj = repo.find_object(oid, None)?;
            Ok(FindObjectOutput {
                oid: oid_str,
                object_type: object_type_label(obj.kind()).to_string(),
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for FindObject {
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

impl TypedOperation for FindObject {
    type Output = FindObjectOutput;
}

#[cfg(test)]
mod tests {
    use git2::Repository;
    use ironflow_core::operation::Operation;

    use super::*;
    use crate::test_helpers::{ctx, init_repo};

    #[tokio::test]
    async fn blob_create_returns_oid_and_size() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        let result = BlobCreate::new(tmp.path(), b"hello".to_vec())
            .run(&ctx())
            .await
            .unwrap();
        assert!(!result.oid.is_empty());
        assert_eq!(result.size, 5);
    }

    #[tokio::test]
    async fn blob_create_empty() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        let result = BlobCreate::new(tmp.path(), vec![])
            .run(&ctx())
            .await
            .unwrap();
        assert_eq!(result.size, 0);
    }

    #[tokio::test]
    async fn tree_lookup_lists_entries() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        let repo = Repository::open(tmp.path()).unwrap();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        let tree_oid = head.tree().unwrap().id().to_string();
        let result = TreeLookup::new(tmp.path(), &tree_oid)
            .run(&ctx())
            .await
            .unwrap();
        assert_eq!(result.oid, tree_oid);
        assert!(result.entries.iter().any(|e| e.name == "file.txt"));
    }

    #[tokio::test]
    async fn find_object_returns_type() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        let repo = Repository::open(tmp.path()).unwrap();
        let commit_oid = repo
            .head()
            .unwrap()
            .peel_to_commit()
            .unwrap()
            .id()
            .to_string();
        let result = FindObject::new(tmp.path(), &commit_oid)
            .run(&ctx())
            .await
            .unwrap();
        assert_eq!(result.object_type, "commit");
    }

    #[tokio::test]
    async fn find_object_invalid_oid_fails() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        assert!(
            FindObject::new(tmp.path(), "0000000000000000000000000000000000000000")
                .run(&ctx())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn execute_serializes_correctly() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        let value = BlobCreate::new(tmp.path(), b"data".to_vec())
            .execute(&ctx())
            .await
            .unwrap();
        assert_eq!(value["size"], 4);
        assert!(value["oid"].is_string());
    }
}
