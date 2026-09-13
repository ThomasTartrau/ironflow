//! Fetch and push operations.

use std::path::PathBuf;

use async_trait::async_trait;
use git2::Repository;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::helpers::{blocking, credentials_callbacks, to_value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchPushOutput {
    pub remote: String,
    pub refspecs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemotePruneOutput {
    pub remote: String,
    pub pruned: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteDefaultBranchOutput {
    pub remote: String,
    pub default_branch: Option<String>,
}

/// Fetch from a remote.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::fetch::FetchRemote;
/// use ironflow_core::operation::Operation;
///
/// let op = FetchRemote::new("/path/to/repo", "origin", vec!["main"]);
/// assert_eq!(op.kind(), "git");
/// ```
pub struct FetchRemote {
    repo_path: PathBuf,
    remote_name: String,
    refspecs: Vec<String>,
}

impl FetchRemote {
    /// Create a new fetch operation.
    pub fn new(
        repo_path: impl Into<PathBuf>,
        remote_name: impl Into<String>,
        refspecs: Vec<impl Into<String>>,
    ) -> Self {
        Self {
            repo_path: repo_path.into(),
            remote_name: remote_name.into(),
            refspecs: refspecs.into_iter().map(Into::into).collect(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<FetchPushOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let remote_name = self.remote_name.clone();
        let refspecs = self.refspecs.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let mut remote = repo.find_remote(&remote_name)?;
            let refs: Vec<&str> = refspecs.iter().map(String::as_str).collect();
            let mut fetch_opts = git2::FetchOptions::new();
            fetch_opts.remote_callbacks(credentials_callbacks());
            remote.fetch(&refs, Some(&mut fetch_opts), None)?;
            Ok(FetchPushOutput {
                remote: remote_name,
                refspecs,
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for FetchRemote {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "repo_path": self.repo_path, "remote": self.remote_name }))
    }
}

impl TypedOperation for FetchRemote {
    type Output = FetchPushOutput;
}

/// Push to a remote.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::fetch::PushRemote;
/// use ironflow_core::operation::Operation;
///
/// let op = PushRemote::new("/path/to/repo", "origin", vec!["refs/heads/main"]);
/// assert_eq!(op.kind(), "git");
/// ```
pub struct PushRemote {
    repo_path: PathBuf,
    remote_name: String,
    refspecs: Vec<String>,
}

impl PushRemote {
    /// Create a new push operation.
    pub fn new(
        repo_path: impl Into<PathBuf>,
        remote_name: impl Into<String>,
        refspecs: Vec<impl Into<String>>,
    ) -> Self {
        Self {
            repo_path: repo_path.into(),
            remote_name: remote_name.into(),
            refspecs: refspecs.into_iter().map(Into::into).collect(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<FetchPushOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let remote_name = self.remote_name.clone();
        let refspecs = self.refspecs.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let mut remote = repo.find_remote(&remote_name)?;
            let refs: Vec<&str> = refspecs.iter().map(String::as_str).collect();
            let mut push_opts = git2::PushOptions::new();
            push_opts.remote_callbacks(credentials_callbacks());
            remote.push(&refs, Some(&mut push_opts))?;
            Ok(FetchPushOutput {
                remote: remote_name,
                refspecs,
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for PushRemote {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "repo_path": self.repo_path, "remote": self.remote_name }))
    }
}

impl TypedOperation for PushRemote {
    type Output = FetchPushOutput;
}

/// Prune stale remote-tracking branches.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::fetch::RemotePrune;
/// use ironflow_core::operation::Operation;
///
/// let op = RemotePrune::new("/path/to/repo", "origin");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct RemotePrune {
    repo_path: PathBuf,
    remote_name: String,
}

impl RemotePrune {
    /// Create a new prune operation.
    pub fn new(repo_path: impl Into<PathBuf>, remote_name: impl Into<String>) -> Self {
        Self {
            repo_path: repo_path.into(),
            remote_name: remote_name.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<RemotePruneOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let remote_name = self.remote_name.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let mut remote = repo.find_remote(&remote_name)?;
            remote.prune(None)?;
            Ok(RemotePruneOutput {
                remote: remote_name,
                pruned: true,
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for RemotePrune {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "repo_path": self.repo_path, "remote": self.remote_name }))
    }
}

impl TypedOperation for RemotePrune {
    type Output = RemotePruneOutput;
}

/// Get the default branch of a remote.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::fetch::RemoteDefaultBranch;
/// use ironflow_core::operation::Operation;
///
/// let op = RemoteDefaultBranch::new("/path/to/repo", "origin");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct RemoteDefaultBranch {
    repo_path: PathBuf,
    remote_name: String,
}

impl RemoteDefaultBranch {
    /// Create a new default-branch query operation.
    pub fn new(repo_path: impl Into<PathBuf>, remote_name: impl Into<String>) -> Self {
        Self {
            repo_path: repo_path.into(),
            remote_name: remote_name.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(
        &self,
        _ctx: &OperationContext,
    ) -> Result<RemoteDefaultBranchOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let remote_name = self.remote_name.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let remote = repo.find_remote(&remote_name)?;
            let default = remote.default_branch()?;
            let name = default.as_str().map(String::from);
            Ok(RemoteDefaultBranchOutput {
                remote: remote_name,
                default_branch: name,
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for RemoteDefaultBranch {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "repo_path": self.repo_path, "remote": self.remote_name }))
    }
}

impl TypedOperation for RemoteDefaultBranch {
    type Output = RemoteDefaultBranchOutput;
}

#[cfg(test)]
mod tests {
    use git2::Repository;

    use super::*;
    use crate::test_helpers::{ctx, init_repo};

    // Push and fetch over a local bare remote (file://) never hit the
    // credentials callback, but exercising them proves that wiring
    // PushOptions/FetchOptions with RemoteCallbacks does not break the normal
    // flow. The SSH-agent path (the literal issue symptom) needs a live SSH
    // server and is covered in Out of Test Scope.
    fn setup_with_bare_remote() -> (tempfile::TempDir, tempfile::TempDir) {
        let work = tempfile::tempdir().unwrap();
        init_repo(work.path());
        let bare = tempfile::tempdir().unwrap();
        Repository::init_bare(bare.path()).unwrap();
        let repo = Repository::open(work.path()).unwrap();
        repo.remote("origin", bare.path().to_str().unwrap())
            .unwrap();
        (work, bare)
    }

    #[tokio::test]
    async fn push_to_local_bare_remote() {
        let (work, bare) = setup_with_bare_remote();
        let result = PushRemote::new(
            work.path(),
            "origin",
            vec!["refs/heads/master:refs/heads/master"],
        )
        .run(&ctx())
        .await
        .unwrap();
        assert_eq!(result.remote, "origin");
        // The ref must exist on the remote side after the push.
        let bare_repo = Repository::open(bare.path()).unwrap();
        assert!(bare_repo.find_reference("refs/heads/master").is_ok());
    }

    #[tokio::test]
    async fn fetch_from_local_bare_remote() {
        let (work, bare) = setup_with_bare_remote();
        PushRemote::new(
            work.path(),
            "origin",
            vec!["refs/heads/master:refs/heads/master"],
        )
        .run(&ctx())
        .await
        .unwrap();
        // A second clone fetching from the same bare remote must succeed.
        let clone_dir = tempfile::tempdir().unwrap();
        let clone = Repository::init(clone_dir.path()).unwrap();
        clone
            .remote("origin", bare.path().to_str().unwrap())
            .unwrap();
        let result = FetchRemote::new(clone_dir.path(), "origin", vec!["master"])
            .run(&ctx())
            .await
            .unwrap();
        assert_eq!(result.remote, "origin");
    }

    #[tokio::test]
    async fn push_missing_remote_fails() {
        let work = tempfile::tempdir().unwrap();
        init_repo(work.path());
        let result = PushRemote::new(work.path(), "nope", vec!["refs/heads/master"])
            .run(&ctx())
            .await;
        assert!(result.is_err());
    }
}
