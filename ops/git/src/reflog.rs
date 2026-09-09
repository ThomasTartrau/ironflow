//! Reflog operations.

use std::path::PathBuf;

use async_trait::async_trait;
use git2::{Oid, Repository, Signature};
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::helpers::{blocking, to_value};

/// A single reflog entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflogEntry {
    pub id_new: String,
    pub id_old: String,
    pub message: String,
    pub committer: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflogReadOutput {
    pub refname: String,
    pub entries: Vec<ReflogEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflogAppendOutput {
    pub refname: String,
    pub appended: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflogDropOutput {
    pub refname: String,
    pub index: usize,
    pub dropped: bool,
}

/// Read the reflog for a reference.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::reflog::ReflogRead;
/// use ironflow_core::operation::Operation;
///
/// let op = ReflogRead::new("/path/to/repo", "HEAD", 50);
/// assert_eq!(op.kind(), "git");
/// ```
pub struct ReflogRead {
    repo_path: PathBuf,
    refname: String,
    limit: usize,
}

impl ReflogRead {
    /// Create a new reflog-read operation.
    pub fn new(repo_path: impl Into<PathBuf>, refname: impl Into<String>, limit: usize) -> Self {
        Self {
            repo_path: repo_path.into(),
            refname: refname.into(),
            limit,
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<ReflogReadOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let refname = self.refname.clone();
        let limit = self.limit;
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let reflog = repo.reflog(&refname)?;
            let entries: Vec<ReflogEntry> = (0..reflog.len().min(limit))
                .filter_map(|i| reflog.get(i))
                .map(|entry| ReflogEntry {
                    id_new: entry.id_new().to_string(),
                    id_old: entry.id_old().to_string(),
                    message: entry.message().unwrap_or("").to_string(),
                    committer: entry.committer().name().unwrap_or("").to_string(),
                })
                .collect();
            Ok(ReflogReadOutput { refname, entries })
        })
        .await
    }
}

#[async_trait]
impl Operation for ReflogRead {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "repo_path": self.repo_path, "refname": self.refname }))
    }
}

impl TypedOperation for ReflogRead {
    type Output = ReflogReadOutput;
}

/// Append an entry to a reflog.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::reflog::ReflogAppend;
/// use ironflow_core::operation::Operation;
///
/// let op = ReflogAppend::new("/path/to/repo", "HEAD", "abc123", "message", "Author", "author@example.com");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct ReflogAppend {
    repo_path: PathBuf,
    refname: String,
    oid: String,
    message: String,
    committer_name: String,
    committer_email: String,
}

impl ReflogAppend {
    /// Create a new reflog-append operation.
    pub fn new(
        repo_path: impl Into<PathBuf>,
        refname: impl Into<String>,
        oid: impl Into<String>,
        message: impl Into<String>,
        committer_name: impl Into<String>,
        committer_email: impl Into<String>,
    ) -> Self {
        Self {
            repo_path: repo_path.into(),
            refname: refname.into(),
            oid: oid.into(),
            message: message.into(),
            committer_name: committer_name.into(),
            committer_email: committer_email.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<ReflogAppendOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let refname = self.refname.clone();
        let oid_str = self.oid.clone();
        let message = self.message.clone();
        let name = self.committer_name.clone();
        let email = self.committer_email.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let mut reflog = repo.reflog(&refname)?;
            let oid = Oid::from_str(&oid_str)?;
            let sig = Signature::now(&name, &email)?;
            reflog.append(oid, &sig, Some(&message))?;
            reflog.write()?;
            Ok(ReflogAppendOutput {
                refname,
                appended: true,
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for ReflogAppend {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "repo_path": self.repo_path, "refname": self.refname }))
    }
}

impl TypedOperation for ReflogAppend {
    type Output = ReflogAppendOutput;
}

/// Drop a reflog entry by index.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::reflog::ReflogDrop;
/// use ironflow_core::operation::Operation;
///
/// let op = ReflogDrop::new("/path/to/repo", "HEAD", 0);
/// assert_eq!(op.kind(), "git");
/// ```
pub struct ReflogDrop {
    repo_path: PathBuf,
    refname: String,
    index: usize,
}

impl ReflogDrop {
    /// Create a new reflog-drop operation.
    pub fn new(repo_path: impl Into<PathBuf>, refname: impl Into<String>, index: usize) -> Self {
        Self {
            repo_path: repo_path.into(),
            refname: refname.into(),
            index,
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<ReflogDropOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let refname = self.refname.clone();
        let index = self.index;
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let mut reflog = repo.reflog(&refname)?;
            reflog.remove(index, true)?;
            reflog.write()?;
            Ok(ReflogDropOutput {
                refname,
                index,
                dropped: true,
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for ReflogDrop {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(
            serde_json::json!({ "repo_path": self.repo_path, "refname": self.refname, "index": self.index }),
        )
    }
}

impl TypedOperation for ReflogDrop {
    type Output = ReflogDropOutput;
}

#[cfg(test)]
mod tests {
    use ironflow_core::operation::Operation;

    use super::*;
    use crate::test_helpers::{ctx, init_repo};

    #[tokio::test]
    async fn read_reflog_has_entries() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        let result = ReflogRead::new(tmp.path(), "HEAD", 50)
            .run(&ctx())
            .await
            .unwrap();
        assert_eq!(result.refname, "HEAD");
        assert!(!result.entries.is_empty());
    }

    #[tokio::test]
    async fn append_and_read() {
        let tmp = tempfile::tempdir().unwrap();
        let oid = init_repo(tmp.path()).to_string();
        let before = ReflogRead::new(tmp.path(), "HEAD", 50)
            .run(&ctx())
            .await
            .unwrap();
        ReflogAppend::new(tmp.path(), "HEAD", &oid, "test entry", "Bot", "bot@t.com")
            .run(&ctx())
            .await
            .unwrap();
        let after = ReflogRead::new(tmp.path(), "HEAD", 50)
            .run(&ctx())
            .await
            .unwrap();
        assert_eq!(after.entries.len(), before.entries.len() + 1);
        assert!(after.entries.iter().any(|e| e.message == "test entry"));
    }

    #[tokio::test]
    async fn drop_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let oid = init_repo(tmp.path()).to_string();
        ReflogAppend::new(tmp.path(), "HEAD", &oid, "to-drop", "Bot", "bot@t.com")
            .run(&ctx())
            .await
            .unwrap();
        let result = ReflogDrop::new(tmp.path(), "HEAD", 0)
            .run(&ctx())
            .await
            .unwrap();
        assert!(result.dropped);
    }

    #[tokio::test]
    async fn execute_serializes_correctly() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        let value = ReflogRead::new(tmp.path(), "HEAD", 50)
            .execute(&ctx())
            .await
            .unwrap();
        assert_eq!(value["refname"], "HEAD");
        assert!(value["entries"].is_array());
    }
}
