//! Branch operations.

use std::path::PathBuf;

use async_trait::async_trait;
use git2::{BranchType, Repository};
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::helpers::{blocking, to_value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchCreateOutput {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchDeleteOutput {
    pub name: String,
    pub deleted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchRenameOutput {
    pub old_name: String,
    pub new_name: String,
}

/// A single branch entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchEntry {
    pub name: String,
    pub is_head: bool,
    #[serde(rename = "type")]
    pub branch_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchListOutput {
    pub branches: Vec<BranchEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchLookupOutput {
    pub name: String,
    pub is_head: bool,
    pub target: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchIsHeadOutput {
    pub name: String,
    pub is_head: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchSetUpstreamOutput {
    pub branch: String,
    pub upstream: String,
}

/// Create a new branch pointing at HEAD.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::branch::BranchCreate;
/// use ironflow_core::operation::Operation;
///
/// let op = BranchCreate::new("/path/to/repo", "feature-x");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct BranchCreate {
    repo_path: PathBuf,
    name: String,
}

impl BranchCreate {
    /// Create a new branch-create operation.
    pub fn new(repo_path: impl Into<PathBuf>, name: impl Into<String>) -> Self {
        Self {
            repo_path: repo_path.into(),
            name: name.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<BranchCreateOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let name = self.name.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let head = repo.head()?.peel_to_commit()?;
            repo.branch(&name, &head, false)?;
            Ok(BranchCreateOutput { name })
        })
        .await
    }
}

#[async_trait]
impl Operation for BranchCreate {
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

impl TypedOperation for BranchCreate {
    type Output = BranchCreateOutput;
}

/// Delete a branch.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::branch::BranchDelete;
/// use ironflow_core::operation::Operation;
///
/// let op = BranchDelete::new("/path/to/repo", "feature-x", false);
/// assert_eq!(op.kind(), "git");
/// ```
pub struct BranchDelete {
    repo_path: PathBuf,
    name: String,
    remote: bool,
}

impl BranchDelete {
    /// Create a new branch-delete operation.
    ///
    /// Set `remote` to `true` to delete a remote-tracking branch.
    pub fn new(repo_path: impl Into<PathBuf>, name: impl Into<String>, remote: bool) -> Self {
        Self {
            repo_path: repo_path.into(),
            name: name.into(),
            remote,
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<BranchDeleteOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let name = self.name.clone();
        let branch_type = if self.remote {
            BranchType::Remote
        } else {
            BranchType::Local
        };
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let mut branch = repo.find_branch(&name, branch_type)?;
            branch.delete()?;
            Ok(BranchDeleteOutput {
                name,
                deleted: true,
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for BranchDelete {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(
            serde_json::json!({ "repo_path": self.repo_path, "name": self.name, "remote": self.remote }),
        )
    }
}

impl TypedOperation for BranchDelete {
    type Output = BranchDeleteOutput;
}

/// Rename a branch.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::branch::BranchRename;
/// use ironflow_core::operation::Operation;
///
/// let op = BranchRename::new("/path/to/repo", "old-name", "new-name", false);
/// assert_eq!(op.kind(), "git");
/// ```
pub struct BranchRename {
    repo_path: PathBuf,
    old_name: String,
    new_name: String,
    force: bool,
}

impl BranchRename {
    /// Create a new branch-rename operation.
    pub fn new(
        repo_path: impl Into<PathBuf>,
        old_name: impl Into<String>,
        new_name: impl Into<String>,
        force: bool,
    ) -> Self {
        Self {
            repo_path: repo_path.into(),
            old_name: old_name.into(),
            new_name: new_name.into(),
            force,
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<BranchRenameOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let old = self.old_name.clone();
        let new = self.new_name.clone();
        let force = self.force;
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let mut branch = repo.find_branch(&old, BranchType::Local)?;
            branch.rename(&new, force)?;
            Ok(BranchRenameOutput {
                old_name: old,
                new_name: new,
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for BranchRename {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "repo_path": self.repo_path,
            "old_name": self.old_name,
            "new_name": self.new_name,
        }))
    }
}

impl TypedOperation for BranchRename {
    type Output = BranchRenameOutput;
}

/// List all branches.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::branch::BranchList;
/// use ironflow_core::operation::Operation;
///
/// let op = BranchList::local("/path/to/repo");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct BranchList {
    repo_path: PathBuf,
    filter: Option<BranchType>,
}

impl BranchList {
    /// List local branches only.
    pub fn local(repo_path: impl Into<PathBuf>) -> Self {
        Self {
            repo_path: repo_path.into(),
            filter: Some(BranchType::Local),
        }
    }

    /// List remote-tracking branches only.
    pub fn remote(repo_path: impl Into<PathBuf>) -> Self {
        Self {
            repo_path: repo_path.into(),
            filter: Some(BranchType::Remote),
        }
    }

    /// List all branches (local and remote).
    pub fn all(repo_path: impl Into<PathBuf>) -> Self {
        Self {
            repo_path: repo_path.into(),
            filter: None,
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<BranchListOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let filter = self.filter;
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let branches = repo.branches(filter)?;
            let list = branches
                .filter_map(|b| b.ok())
                .map(|(branch, bt)| BranchEntry {
                    name: branch.name().ok().flatten().unwrap_or("").to_string(),
                    is_head: branch.is_head(),
                    branch_type: match bt {
                        BranchType::Local => "local".to_string(),
                        BranchType::Remote => "remote".to_string(),
                    },
                })
                .collect();
            Ok(BranchListOutput { branches: list })
        })
        .await
    }
}

#[async_trait]
impl Operation for BranchList {
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

impl TypedOperation for BranchList {
    type Output = BranchListOutput;
}

/// Look up a branch by name.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::branch::BranchLookup;
/// use ironflow_core::operation::Operation;
///
/// let op = BranchLookup::new("/path/to/repo", "main", false);
/// assert_eq!(op.kind(), "git");
/// ```
pub struct BranchLookup {
    repo_path: PathBuf,
    name: String,
    remote: bool,
}

impl BranchLookup {
    /// Create a new branch-lookup operation.
    pub fn new(repo_path: impl Into<PathBuf>, name: impl Into<String>, remote: bool) -> Self {
        Self {
            repo_path: repo_path.into(),
            name: name.into(),
            remote,
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<BranchLookupOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let name = self.name.clone();
        let bt = if self.remote {
            BranchType::Remote
        } else {
            BranchType::Local
        };
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let branch = repo.find_branch(&name, bt)?;
            let target = branch.get().target().map(|o| o.to_string());
            Ok(BranchLookupOutput {
                name,
                is_head: branch.is_head(),
                target,
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for BranchLookup {
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

impl TypedOperation for BranchLookup {
    type Output = BranchLookupOutput;
}

/// Check if a branch is the current HEAD.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::branch::BranchIsHead;
/// use ironflow_core::operation::Operation;
///
/// let op = BranchIsHead::new("/path/to/repo", "main");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct BranchIsHead {
    repo_path: PathBuf,
    name: String,
}

impl BranchIsHead {
    /// Create a new is-head check operation.
    pub fn new(repo_path: impl Into<PathBuf>, name: impl Into<String>) -> Self {
        Self {
            repo_path: repo_path.into(),
            name: name.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<BranchIsHeadOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let name = self.name.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let branch = repo.find_branch(&name, BranchType::Local)?;
            Ok(BranchIsHeadOutput {
                name,
                is_head: branch.is_head(),
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for BranchIsHead {
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

impl TypedOperation for BranchIsHead {
    type Output = BranchIsHeadOutput;
}

/// Set the upstream for a branch.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::branch::BranchSetUpstream;
/// use ironflow_core::operation::Operation;
///
/// let op = BranchSetUpstream::new("/path/to/repo", "main", "origin/main");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct BranchSetUpstream {
    repo_path: PathBuf,
    branch_name: String,
    upstream_name: String,
}

impl BranchSetUpstream {
    /// Create a new set-upstream operation.
    pub fn new(
        repo_path: impl Into<PathBuf>,
        branch_name: impl Into<String>,
        upstream_name: impl Into<String>,
    ) -> Self {
        Self {
            repo_path: repo_path.into(),
            branch_name: branch_name.into(),
            upstream_name: upstream_name.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(
        &self,
        _ctx: &OperationContext,
    ) -> Result<BranchSetUpstreamOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let branch_name = self.branch_name.clone();
        let upstream = self.upstream_name.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let mut branch = repo.find_branch(&branch_name, BranchType::Local)?;
            branch.set_upstream(Some(&upstream))?;
            Ok(BranchSetUpstreamOutput {
                branch: branch_name,
                upstream,
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for BranchSetUpstream {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "repo_path": self.repo_path,
            "branch": self.branch_name,
            "upstream": self.upstream_name,
        }))
    }
}

impl TypedOperation for BranchSetUpstream {
    type Output = BranchSetUpstreamOutput;
}
