//! Repository-level operations: init, open, clone, discover, state.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use git2::{Repository, RepositoryState};
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::helpers::{blocking, to_value};

fn repo_state_label(state: RepositoryState) -> &'static str {
    match state {
        RepositoryState::Clean => "clean",
        RepositoryState::Merge => "merge",
        RepositoryState::Revert | RepositoryState::RevertSequence => "revert",
        RepositoryState::CherryPickSequence | RepositoryState::CherryPick => "cherrypick",
        RepositoryState::Bisect => "bisect",
        RepositoryState::Rebase
        | RepositoryState::RebaseInteractive
        | RepositoryState::RebaseMerge => "rebase",
        RepositoryState::ApplyMailbox | RepositoryState::ApplyMailboxOrRebase => "apply-mailbox",
    }
}

/// Output of [`RepoInit`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoInitOutput {
    /// Path where the repository was created.
    pub path: PathBuf,
    /// Whether this is a bare repository.
    pub bare: bool,
}

/// Initialize a new Git repository.
///
/// Creates a new repository at the given path. If `bare` is true, creates
/// a bare repository (no working directory).
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::repository::RepoInit;
/// use ironflow_core::operation::Operation;
///
/// let op = RepoInit::new("/tmp/my-repo", false);
/// assert_eq!(op.kind(), "git");
/// ```
pub struct RepoInit {
    path: PathBuf,
    bare: bool,
}

impl RepoInit {
    /// Create a new init operation.
    pub fn new(path: impl Into<PathBuf>, bare: bool) -> Self {
        Self {
            path: path.into(),
            bare,
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<RepoInitOutput, OperationError> {
        let path = self.path.clone();
        let bare = self.bare;
        blocking(move || {
            if bare {
                Repository::init_bare(&path)?;
            } else {
                Repository::init(&path)?;
            }
            Ok(RepoInitOutput { path, bare })
        })
        .await
    }
}

#[async_trait]
impl Operation for RepoInit {
    fn kind(&self) -> &str {
        "git"
    }

    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "path": self.path, "bare": self.bare }))
    }
}

impl TypedOperation for RepoInit {
    type Output = RepoInitOutput;
}

/// Output of [`RepoOpen`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoOpenOutput {
    /// Working directory path (or the bare repo path).
    pub path: PathBuf,
    /// Whether this is a bare repository.
    pub bare: bool,
}

/// Open an existing Git repository.
///
/// Returns the repository path and whether it is bare.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::repository::RepoOpen;
/// use ironflow_core::operation::Operation;
///
/// let op = RepoOpen::new("/path/to/repo");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct RepoOpen {
    path: PathBuf,
}

impl RepoOpen {
    /// Create a new open operation.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<RepoOpenOutput, OperationError> {
        let path = self.path.clone();
        blocking(move || {
            let repo = Repository::open(&path)?;
            let is_bare = repo.is_bare();
            let workdir = repo.workdir().map(Path::to_path_buf);
            Ok(RepoOpenOutput {
                path: workdir.unwrap_or(path),
                bare: is_bare,
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for RepoOpen {
    fn kind(&self) -> &str {
        "git"
    }

    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "path": self.path }))
    }
}

impl TypedOperation for RepoOpen {
    type Output = RepoOpenOutput;
}

/// Output of [`RepoClone`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoCloneOutput {
    /// The cloned URL.
    pub url: String,
    /// Local path of the clone.
    pub path: PathBuf,
}

/// Clone a remote or local repository.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::repository::RepoClone;
/// use ironflow_core::operation::Operation;
///
/// let op = RepoClone::new("https://github.com/user/repo.git", "/tmp/clone");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct RepoClone {
    url: String,
    path: PathBuf,
}

impl RepoClone {
    /// Create a new clone operation.
    pub fn new(url: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self {
            url: url.into(),
            path: path.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<RepoCloneOutput, OperationError> {
        let url = self.url.clone();
        let path = self.path.clone();
        blocking(move || {
            Repository::clone(&url, &path)?;
            Ok(RepoCloneOutput { url, path })
        })
        .await
    }
}

#[async_trait]
impl Operation for RepoClone {
    fn kind(&self) -> &str {
        "git"
    }

    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "url": self.url, "path": self.path }))
    }
}

impl TypedOperation for RepoClone {
    type Output = RepoCloneOutput;
}

/// Output of [`RepoDiscover`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoDiscoverOutput {
    /// Working directory path (if not bare).
    pub path: Option<PathBuf>,
    /// Whether this is a bare repository.
    pub bare: bool,
}

/// Discover a repository by walking parent directories.
///
/// Starts from `start_path` and walks upward until a `.git` directory is found.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::repository::RepoDiscover;
/// use ironflow_core::operation::Operation;
///
/// let op = RepoDiscover::new("/path/to/subdir");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct RepoDiscover {
    start_path: PathBuf,
}

impl RepoDiscover {
    /// Create a new discover operation.
    pub fn new(start_path: impl Into<PathBuf>) -> Self {
        Self {
            start_path: start_path.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<RepoDiscoverOutput, OperationError> {
        let start = self.start_path.clone();
        blocking(move || {
            let repo = Repository::discover(&start)?;
            let workdir = repo.workdir().map(Path::to_path_buf);
            let is_bare = repo.is_bare();
            Ok(RepoDiscoverOutput {
                path: workdir,
                bare: is_bare,
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for RepoDiscover {
    fn kind(&self) -> &str {
        "git"
    }

    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "start_path": self.start_path }))
    }
}

impl TypedOperation for RepoDiscover {
    type Output = RepoDiscoverOutput;
}

/// Output of [`RepoState`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoStateOutput {
    /// Repository state (e.g. "Clean", "Merge", "Rebase").
    pub state: String,
}

/// Query the current state of the repository.
///
/// Returns the repository state (clean, merge, rebase, etc.).
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::repository::RepoState;
/// use ironflow_core::operation::Operation;
///
/// let op = RepoState::new("/path/to/repo");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct RepoState {
    repo_path: PathBuf,
}

impl RepoState {
    /// Create a new state query operation.
    pub fn new(repo_path: impl Into<PathBuf>) -> Self {
        Self {
            repo_path: repo_path.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<RepoStateOutput, OperationError> {
        let path = self.repo_path.clone();
        blocking(move || {
            let repo = Repository::open(&path)?;
            let state = repo_state_label(repo.state()).to_string();
            Ok(RepoStateOutput { state })
        })
        .await
    }
}

#[async_trait]
impl Operation for RepoState {
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

impl TypedOperation for RepoState {
    type Output = RepoStateOutput;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::ctx;

    #[tokio::test]
    async fn init_creates_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("new-repo");
        let op = RepoInit::new(&target, false);
        let result = op.run(&ctx()).await.unwrap();
        assert!(!result.bare);
        assert!(target.join(".git").exists());
    }

    #[tokio::test]
    async fn init_creates_bare_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("bare-repo");
        let op = RepoInit::new(&target, true);
        let result = op.run(&ctx()).await.unwrap();
        assert!(result.bare);
        assert!(target.join("HEAD").exists());
    }

    #[tokio::test]
    async fn clone_local() {
        let tmp = tempfile::tempdir().unwrap();
        let origin = tmp.path().join("origin");
        Repository::init(&origin).unwrap();

        let target = tmp.path().join("clone");
        let url = origin.to_str().unwrap();
        let op = RepoClone::new(url, &target);
        let result = op.run(&ctx()).await.unwrap();
        assert_eq!(result.path, target);
        assert!(target.join(".git").exists());
    }

    #[tokio::test]
    async fn discover_finds_repo() {
        let tmp = tempfile::tempdir().unwrap();
        Repository::init(tmp.path()).unwrap();
        let subdir = tmp.path().join("a").join("b");
        std::fs::create_dir_all(&subdir).unwrap();

        let op = RepoDiscover::new(&subdir);
        let result = op.run(&ctx()).await.unwrap();
        assert!(!result.bare);
    }

    #[tokio::test]
    async fn state_on_clean_repo() {
        let tmp = tempfile::tempdir().unwrap();
        Repository::init(tmp.path()).unwrap();

        let op = RepoState::new(tmp.path());
        let result = op.run(&ctx()).await.unwrap();
        assert_eq!(result.state, "clean");
    }
}
