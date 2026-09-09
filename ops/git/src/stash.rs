//! Stash operations.

use std::path::PathBuf;

use async_trait::async_trait;
use git2::{Repository, Signature};
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::helpers::{blocking, to_value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StashSaveOutput {
    pub oid: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StashApplyOutput {
    pub index: usize,
    pub applied: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StashPopOutput {
    pub index: usize,
    pub popped: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StashDropOutput {
    pub index: usize,
    pub dropped: bool,
}

/// A single stash entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StashEntry {
    pub index: usize,
    pub message: String,
    pub oid: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StashListOutput {
    pub stashes: Vec<StashEntry>,
}

/// Save changes to the stash.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::stash::StashSave;
/// use ironflow_core::operation::Operation;
///
/// let op = StashSave::new("/path/to/repo", "Test", "test@example.com", "WIP");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct StashSave {
    repo_path: PathBuf,
    stasher_name: String,
    stasher_email: String,
    message: Option<String>,
}

impl StashSave {
    /// Create a new stash-save operation.
    pub fn new(
        repo_path: impl Into<PathBuf>,
        stasher_name: impl Into<String>,
        stasher_email: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            repo_path: repo_path.into(),
            stasher_name: stasher_name.into(),
            stasher_email: stasher_email.into(),
            message: Some(message.into()),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<StashSaveOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let name = self.stasher_name.clone();
        let email = self.stasher_email.clone();
        let message = self.message.clone();
        blocking(move || {
            let mut repo = Repository::open(&repo_path)?;
            let sig = Signature::now(&name, &email)?;
            let oid = repo.stash_save(&sig, message.as_deref().unwrap_or(""), None)?;
            Ok(StashSaveOutput {
                oid: oid.to_string(),
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for StashSave {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "repo_path": self.repo_path, "message": self.message }))
    }
}

impl TypedOperation for StashSave {
    type Output = StashSaveOutput;
}

/// Apply a stash entry.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::stash::StashApply;
/// use ironflow_core::operation::Operation;
///
/// let op = StashApply::new("/path/to/repo", 0);
/// assert_eq!(op.kind(), "git");
/// ```
pub struct StashApply {
    repo_path: PathBuf,
    index: usize,
}

impl StashApply {
    /// Create a new stash-apply operation.
    pub fn new(repo_path: impl Into<PathBuf>, index: usize) -> Self {
        Self {
            repo_path: repo_path.into(),
            index,
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<StashApplyOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let index = self.index;
        blocking(move || {
            let mut repo = Repository::open(&repo_path)?;
            repo.stash_apply(index, None)?;
            Ok(StashApplyOutput {
                index,
                applied: true,
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for StashApply {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "repo_path": self.repo_path, "index": self.index }))
    }
}

impl TypedOperation for StashApply {
    type Output = StashApplyOutput;
}

/// Pop a stash entry (apply and drop).
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::stash::StashPop;
/// use ironflow_core::operation::Operation;
///
/// let op = StashPop::new("/path/to/repo", 0);
/// assert_eq!(op.kind(), "git");
/// ```
pub struct StashPop {
    repo_path: PathBuf,
    index: usize,
}

impl StashPop {
    /// Create a new stash-pop operation.
    pub fn new(repo_path: impl Into<PathBuf>, index: usize) -> Self {
        Self {
            repo_path: repo_path.into(),
            index,
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<StashPopOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let index = self.index;
        blocking(move || {
            let mut repo = Repository::open(&repo_path)?;
            repo.stash_pop(index, None)?;
            Ok(StashPopOutput {
                index,
                popped: true,
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for StashPop {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "repo_path": self.repo_path, "index": self.index }))
    }
}

impl TypedOperation for StashPop {
    type Output = StashPopOutput;
}

/// Drop a stash entry.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::stash::StashDrop;
/// use ironflow_core::operation::Operation;
///
/// let op = StashDrop::new("/path/to/repo", 0);
/// assert_eq!(op.kind(), "git");
/// ```
pub struct StashDrop {
    repo_path: PathBuf,
    index: usize,
}

impl StashDrop {
    /// Create a new stash-drop operation.
    pub fn new(repo_path: impl Into<PathBuf>, index: usize) -> Self {
        Self {
            repo_path: repo_path.into(),
            index,
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<StashDropOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let index = self.index;
        blocking(move || {
            let mut repo = Repository::open(&repo_path)?;
            repo.stash_drop(index)?;
            Ok(StashDropOutput {
                index,
                dropped: true,
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for StashDrop {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "repo_path": self.repo_path, "index": self.index }))
    }
}

impl TypedOperation for StashDrop {
    type Output = StashDropOutput;
}

/// List all stash entries.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::stash::StashList;
/// use ironflow_core::operation::Operation;
///
/// let op = StashList::new("/path/to/repo");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct StashList {
    repo_path: PathBuf,
}

impl StashList {
    /// Create a new stash-list operation.
    pub fn new(repo_path: impl Into<PathBuf>) -> Self {
        Self {
            repo_path: repo_path.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<StashListOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        blocking(move || {
            let mut repo = Repository::open(&repo_path)?;
            let mut entries = Vec::new();
            repo.stash_foreach(|index, message, oid| {
                entries.push(StashEntry {
                    index,
                    message: message.to_string(),
                    oid: oid.to_string(),
                });
                true
            })?;
            Ok(StashListOutput { stashes: entries })
        })
        .await
    }
}

#[async_trait]
impl Operation for StashList {
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

impl TypedOperation for StashList {
    type Output = StashListOutput;
}

#[cfg(test)]
mod tests {
    use std::fs;

    use ironflow_core::operation::Operation;

    use super::*;
    use crate::test_helpers::{ctx, init_repo};

    #[tokio::test]
    async fn save_and_list() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        fs::write(tmp.path().join("file.txt"), "dirty").unwrap();
        let result = StashSave::new(tmp.path(), "Test", "t@t.com", "WIP")
            .run(&ctx())
            .await
            .unwrap();
        assert!(!result.oid.is_empty());
        assert_eq!(
            fs::read_to_string(tmp.path().join("file.txt")).unwrap(),
            "content"
        );
        let list = StashList::new(tmp.path()).run(&ctx()).await.unwrap();
        assert_eq!(list.stashes.len(), 1);
    }

    #[tokio::test]
    async fn save_nothing_to_stash_fails() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        assert!(
            StashSave::new(tmp.path(), "Test", "t@t.com", "WIP")
                .run(&ctx())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn apply_restores_changes() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        fs::write(tmp.path().join("file.txt"), "dirty").unwrap();
        StashSave::new(tmp.path(), "Test", "t@t.com", "WIP")
            .run(&ctx())
            .await
            .unwrap();
        let result = StashApply::new(tmp.path(), 0).run(&ctx()).await.unwrap();
        assert!(result.applied);
        assert_eq!(
            fs::read_to_string(tmp.path().join("file.txt")).unwrap(),
            "dirty"
        );
    }

    #[tokio::test]
    async fn pop_removes_from_list() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        fs::write(tmp.path().join("file.txt"), "dirty").unwrap();
        StashSave::new(tmp.path(), "Test", "t@t.com", "WIP")
            .run(&ctx())
            .await
            .unwrap();
        StashPop::new(tmp.path(), 0).run(&ctx()).await.unwrap();
        let list = StashList::new(tmp.path()).run(&ctx()).await.unwrap();
        assert!(list.stashes.is_empty());
    }

    #[tokio::test]
    async fn drop_removes_entry() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        fs::write(tmp.path().join("file.txt"), "dirty").unwrap();
        StashSave::new(tmp.path(), "Test", "t@t.com", "WIP")
            .run(&ctx())
            .await
            .unwrap();
        StashDrop::new(tmp.path(), 0).run(&ctx()).await.unwrap();
        let list = StashList::new(tmp.path()).run(&ctx()).await.unwrap();
        assert!(list.stashes.is_empty());
    }

    #[tokio::test]
    async fn execute_serializes_correctly() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        let value = StashList::new(tmp.path()).execute(&ctx()).await.unwrap();
        assert!(value["stashes"].as_array().unwrap().is_empty());
    }
}
