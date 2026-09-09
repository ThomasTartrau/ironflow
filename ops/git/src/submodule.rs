//! Submodule operations.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use git2::Repository;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::helpers::{blocking, to_value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmoduleAddOutput {
    pub url: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmoduleInitOutput {
    pub name: String,
    pub initialized: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmoduleUpdateOutput {
    pub name: String,
    pub updated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmoduleLookupOutput {
    pub name: String,
    pub url: String,
    pub path: String,
    pub head_id: Option<String>,
}

/// A single submodule entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmoduleEntry {
    pub name: String,
    pub url: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmoduleListOutput {
    pub submodules: Vec<SubmoduleEntry>,
}

/// Add a submodule.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::submodule::SubmoduleAdd;
/// use ironflow_core::operation::Operation;
///
/// let op = SubmoduleAdd::new("/path/to/repo", "https://example.com/sub.git", "vendor/sub");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct SubmoduleAdd {
    repo_path: PathBuf,
    url: String,
    path: String,
}

impl SubmoduleAdd {
    /// Create a new submodule-add operation.
    pub fn new(
        repo_path: impl Into<PathBuf>,
        url: impl Into<String>,
        path: impl Into<String>,
    ) -> Self {
        Self {
            repo_path: repo_path.into(),
            url: url.into(),
            path: path.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<SubmoduleAddOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let url = self.url.clone();
        let path = self.path.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            repo.submodule(&url, Path::new(&path), true)?;
            Ok(SubmoduleAddOutput { url, path })
        })
        .await
    }
}

#[async_trait]
impl Operation for SubmoduleAdd {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "repo_path": self.repo_path, "url": self.url, "path": self.path }))
    }
}

impl TypedOperation for SubmoduleAdd {
    type Output = SubmoduleAddOutput;
}

/// Initialize a submodule.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::submodule::SubmoduleInit;
/// use ironflow_core::operation::Operation;
///
/// let op = SubmoduleInit::new("/path/to/repo", "vendor/sub");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct SubmoduleInit {
    repo_path: PathBuf,
    name: String,
}

impl SubmoduleInit {
    /// Create a new submodule-init operation.
    pub fn new(repo_path: impl Into<PathBuf>, name: impl Into<String>) -> Self {
        Self {
            repo_path: repo_path.into(),
            name: name.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(
        &self,
        _ctx: &OperationContext,
    ) -> Result<SubmoduleInitOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let name = self.name.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let mut sub = repo.find_submodule(&name)?;
            sub.init(false)?;
            Ok(SubmoduleInitOutput {
                name,
                initialized: true,
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for SubmoduleInit {
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

impl TypedOperation for SubmoduleInit {
    type Output = SubmoduleInitOutput;
}

/// Update a submodule (clone or fetch + checkout).
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::submodule::SubmoduleUpdate;
/// use ironflow_core::operation::Operation;
///
/// let op = SubmoduleUpdate::new("/path/to/repo", "vendor/sub");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct SubmoduleUpdate {
    repo_path: PathBuf,
    name: String,
}

impl SubmoduleUpdate {
    /// Create a new submodule-update operation.
    pub fn new(repo_path: impl Into<PathBuf>, name: impl Into<String>) -> Self {
        Self {
            repo_path: repo_path.into(),
            name: name.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(
        &self,
        _ctx: &OperationContext,
    ) -> Result<SubmoduleUpdateOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let name = self.name.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let mut sub = repo.find_submodule(&name)?;
            sub.update(true, None)?;
            Ok(SubmoduleUpdateOutput {
                name,
                updated: true,
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for SubmoduleUpdate {
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

impl TypedOperation for SubmoduleUpdate {
    type Output = SubmoduleUpdateOutput;
}

/// Look up a submodule by name.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::submodule::SubmoduleLookup;
/// use ironflow_core::operation::Operation;
///
/// let op = SubmoduleLookup::new("/path/to/repo", "vendor/sub");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct SubmoduleLookup {
    repo_path: PathBuf,
    name: String,
}

impl SubmoduleLookup {
    /// Create a new submodule-lookup operation.
    pub fn new(repo_path: impl Into<PathBuf>, name: impl Into<String>) -> Self {
        Self {
            repo_path: repo_path.into(),
            name: name.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(
        &self,
        _ctx: &OperationContext,
    ) -> Result<SubmoduleLookupOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let name = self.name.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let sub = repo.find_submodule(&name)?;
            Ok(SubmoduleLookupOutput {
                name: sub.name().unwrap_or("").to_string(),
                url: sub.url().unwrap_or("").to_string(),
                path: sub.path().to_string_lossy().into_owned(),
                head_id: sub.head_id().map(|o| o.to_string()),
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for SubmoduleLookup {
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

impl TypedOperation for SubmoduleLookup {
    type Output = SubmoduleLookupOutput;
}

/// List all submodules.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::submodule::SubmoduleList;
/// use ironflow_core::operation::Operation;
///
/// let op = SubmoduleList::new("/path/to/repo");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct SubmoduleList {
    repo_path: PathBuf,
}

impl SubmoduleList {
    /// Create a new submodule-list operation.
    pub fn new(repo_path: impl Into<PathBuf>) -> Self {
        Self {
            repo_path: repo_path.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(
        &self,
        _ctx: &OperationContext,
    ) -> Result<SubmoduleListOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let subs = repo.submodules()?;
            let list = subs
                .iter()
                .map(|s| SubmoduleEntry {
                    name: s.name().unwrap_or("").to_string(),
                    url: s.url().unwrap_or("").to_string(),
                    path: s.path().to_string_lossy().into_owned(),
                })
                .collect();
            Ok(SubmoduleListOutput { submodules: list })
        })
        .await
    }
}

#[async_trait]
impl Operation for SubmoduleList {
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

impl TypedOperation for SubmoduleList {
    type Output = SubmoduleListOutput;
}
