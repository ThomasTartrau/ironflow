//! Reference operations.

use std::path::PathBuf;

use async_trait::async_trait;
use git2::{Oid, ReferenceType, Repository};
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::helpers::{blocking, to_value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefCreateOutput {
    pub name: String,
    pub target: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefDeleteOutput {
    pub name: String,
    pub deleted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefRenameOutput {
    pub old_name: String,
    pub new_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefLookupOutput {
    pub name: String,
    pub target: Option<String>,
    pub symbolic: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefNameToIdOutput {
    pub name: String,
    pub oid: String,
}

/// Create a new reference.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::refs::RefCreate;
/// use ironflow_core::operation::Operation;
///
/// let op = RefCreate::new("/path/to/repo", "refs/heads/new-branch", "abc123", "create branch");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct RefCreate {
    repo_path: PathBuf,
    name: String,
    target: String,
    log_message: String,
}

impl RefCreate {
    /// Create a new reference-create operation.
    pub fn new(
        repo_path: impl Into<PathBuf>,
        name: impl Into<String>,
        target: impl Into<String>,
        log_message: impl Into<String>,
    ) -> Self {
        Self {
            repo_path: repo_path.into(),
            name: name.into(),
            target: target.into(),
            log_message: log_message.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<RefCreateOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let name = self.name.clone();
        let target = self.target.clone();
        let msg = self.log_message.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let oid = Oid::from_str(&target)?;
            repo.reference(&name, oid, false, &msg)?;
            Ok(RefCreateOutput { name, target })
        })
        .await
    }
}

#[async_trait]
impl Operation for RefCreate {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(
            serde_json::json!({ "repo_path": self.repo_path, "name": self.name, "target": self.target }),
        )
    }
}

impl TypedOperation for RefCreate {
    type Output = RefCreateOutput;
}

/// Delete a reference.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::refs::RefDelete;
/// use ironflow_core::operation::Operation;
///
/// let op = RefDelete::new("/path/to/repo", "refs/heads/old-branch");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct RefDelete {
    repo_path: PathBuf,
    name: String,
}

impl RefDelete {
    /// Create a new reference-delete operation.
    pub fn new(repo_path: impl Into<PathBuf>, name: impl Into<String>) -> Self {
        Self {
            repo_path: repo_path.into(),
            name: name.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<RefDeleteOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let name = self.name.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let mut reference = repo.find_reference(&name)?;
            reference.delete()?;
            Ok(RefDeleteOutput {
                name,
                deleted: true,
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for RefDelete {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "repo_path": self.repo_path, "name": self.name }))
    }
}

impl TypedOperation for RefDelete {
    type Output = RefDeleteOutput;
}

/// Rename a reference.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::refs::RefRename;
/// use ironflow_core::operation::Operation;
///
/// let op = RefRename::new("/path/to/repo", "refs/heads/old", "refs/heads/new", "rename", false);
/// assert_eq!(op.kind(), "git");
/// ```
pub struct RefRename {
    repo_path: PathBuf,
    old_name: String,
    new_name: String,
    log_message: String,
    force: bool,
}

impl RefRename {
    /// Create a new reference-rename operation.
    pub fn new(
        repo_path: impl Into<PathBuf>,
        old_name: impl Into<String>,
        new_name: impl Into<String>,
        log_message: impl Into<String>,
        force: bool,
    ) -> Self {
        Self {
            repo_path: repo_path.into(),
            old_name: old_name.into(),
            new_name: new_name.into(),
            log_message: log_message.into(),
            force,
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<RefRenameOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let old = self.old_name.clone();
        let new = self.new_name.clone();
        let msg = self.log_message.clone();
        let force = self.force;
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let mut reference = repo.find_reference(&old)?;
            reference.rename(&new, force, &msg)?;
            Ok(RefRenameOutput {
                old_name: old,
                new_name: new,
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for RefRename {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(
            serde_json::json!({ "repo_path": self.repo_path, "old_name": self.old_name, "new_name": self.new_name }),
        )
    }
}

impl TypedOperation for RefRename {
    type Output = RefRenameOutput;
}

/// Look up a reference by name.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::refs::RefLookup;
/// use ironflow_core::operation::Operation;
///
/// let op = RefLookup::new("/path/to/repo", "refs/heads/main");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct RefLookup {
    repo_path: PathBuf,
    name: String,
}

impl RefLookup {
    /// Create a new reference-lookup operation.
    pub fn new(repo_path: impl Into<PathBuf>, name: impl Into<String>) -> Self {
        Self {
            repo_path: repo_path.into(),
            name: name.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<RefLookupOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let name = self.name.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let reference = repo.find_reference(&name)?;
            let target = reference.target().map(|o| o.to_string());
            let is_symbolic = reference.kind() == Some(ReferenceType::Symbolic);
            Ok(RefLookupOutput {
                name,
                target,
                symbolic: is_symbolic,
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for RefLookup {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "repo_path": self.repo_path, "name": self.name }))
    }
}

impl TypedOperation for RefLookup {
    type Output = RefLookupOutput;
}

/// Resolve a reference name to an OID.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::refs::RefNameToId;
/// use ironflow_core::operation::Operation;
///
/// let op = RefNameToId::new("/path/to/repo", "HEAD");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct RefNameToId {
    repo_path: PathBuf,
    name: String,
}

impl RefNameToId {
    /// Create a new name-to-id operation.
    pub fn new(repo_path: impl Into<PathBuf>, name: impl Into<String>) -> Self {
        Self {
            repo_path: repo_path.into(),
            name: name.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<RefNameToIdOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let name = self.name.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let oid = repo.refname_to_id(&name)?;
            Ok(RefNameToIdOutput {
                name,
                oid: oid.to_string(),
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for RefNameToId {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "repo_path": self.repo_path, "name": self.name }))
    }
}

impl TypedOperation for RefNameToId {
    type Output = RefNameToIdOutput;
}

#[cfg(test)]
mod tests {
    use ironflow_core::operation::Operation;

    use super::*;
    use crate::test_helpers::{ctx, init_repo};

    #[tokio::test]
    async fn create_and_lookup() {
        let tmp = tempfile::tempdir().unwrap();
        let oid = init_repo(tmp.path()).to_string();
        RefCreate::new(tmp.path(), "refs/heads/test-ref", &oid, "create")
            .run(&ctx())
            .await
            .unwrap();
        let result = RefLookup::new(tmp.path(), "refs/heads/test-ref")
            .run(&ctx())
            .await
            .unwrap();
        assert_eq!(result.name, "refs/heads/test-ref");
        assert_eq!(result.target.as_deref(), Some(oid.as_str()));
        assert!(!result.symbolic);
    }

    #[tokio::test]
    async fn name_to_id() {
        let tmp = tempfile::tempdir().unwrap();
        let oid = init_repo(tmp.path()).to_string();
        let result = RefNameToId::new(tmp.path(), "HEAD")
            .run(&ctx())
            .await
            .unwrap();
        assert_eq!(result.oid, oid);
    }

    #[tokio::test]
    async fn rename_ref() {
        let tmp = tempfile::tempdir().unwrap();
        let oid = init_repo(tmp.path()).to_string();
        RefCreate::new(tmp.path(), "refs/heads/old-ref", &oid, "c")
            .run(&ctx())
            .await
            .unwrap();
        let result = RefRename::new(
            tmp.path(),
            "refs/heads/old-ref",
            "refs/heads/new-ref",
            "rename",
            false,
        )
        .run(&ctx())
        .await
        .unwrap();
        assert_eq!(result.old_name, "refs/heads/old-ref");
        assert_eq!(result.new_name, "refs/heads/new-ref");
        assert!(
            RefLookup::new(tmp.path(), "refs/heads/new-ref")
                .run(&ctx())
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn delete_ref() {
        let tmp = tempfile::tempdir().unwrap();
        let oid = init_repo(tmp.path()).to_string();
        RefCreate::new(tmp.path(), "refs/heads/to-delete", &oid, "c")
            .run(&ctx())
            .await
            .unwrap();
        let result = RefDelete::new(tmp.path(), "refs/heads/to-delete")
            .run(&ctx())
            .await
            .unwrap();
        assert!(result.deleted);
        assert!(
            RefLookup::new(tmp.path(), "refs/heads/to-delete")
                .run(&ctx())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn lookup_missing_ref_fails() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        assert!(
            RefLookup::new(tmp.path(), "refs/heads/nope")
                .run(&ctx())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn execute_serializes_correctly() {
        let tmp = tempfile::tempdir().unwrap();
        let oid = init_repo(tmp.path()).to_string();
        let value = RefNameToId::new(tmp.path(), "HEAD")
            .execute(&ctx())
            .await
            .unwrap();
        assert_eq!(value["oid"], oid);
    }
}
