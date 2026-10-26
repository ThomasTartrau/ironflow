//! Add a worktree with the git CLI, partial clones included.

use std::path::{PathBuf, absolute};

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::cli::command::{git_command, run_git, validate_ref_name};
use crate::helpers::{GitAuth, auth_builders, external_error, to_value, token_builder};
use crate::repo_lock::lock_repo;

/// Output of [`CliWorktreeAdd`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliWorktreeAddOutput {
    /// Repository the worktree was added to.
    pub repo_path: PathBuf,
    /// Path of the new worktree.
    pub path: PathBuf,
    /// SHA the worktree `HEAD` points to.
    pub commit: String,
    /// Branch created for the worktree, `None` when it is detached.
    pub branch: Option<String>,
}

/// Add a worktree with the git CLI.
///
/// Works on a partial clone made by [`CliClone`](crate::cli::CliClone),
/// which libgit2 cannot open. The checkout downloads the missing file
/// contents from the remote, so the operation authenticates like a fetch.
///
/// Without [`CliWorktreeAdd::branch`], the worktree `HEAD` is detached at
/// `commitish`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::cli::CliWorktreeAdd;
/// use ironflow_core::operation::{OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// # let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let wt = CliWorktreeAdd::new("/tmp/repo", "/tmp/mr-42", "origin/main")
///     .branch("mr-42")
///     .run(&ctx)
///     .await?;
/// println!("worktree at {}", wt.commit);
/// # Ok(())
/// # }
/// ```
pub struct CliWorktreeAdd {
    repo_path: PathBuf,
    path: PathBuf,
    commitish: String,
    branch: Option<String>,
    auth: GitAuth,
}

impl CliWorktreeAdd {
    /// Add a worktree at `path` to the repository at `repo_path`, checked
    /// out at `commitish` (a SHA, branch, tag or any revision git accepts).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_git::cli::CliWorktreeAdd;
    /// use ironflow_core::operation::Operation;
    ///
    /// let op = CliWorktreeAdd::new("/tmp/repo", "/tmp/wt", "main");
    /// assert_eq!(op.kind(), "git");
    /// ```
    pub fn new(
        repo_path: impl Into<PathBuf>,
        path: impl Into<PathBuf>,
        commitish: impl Into<String>,
    ) -> Self {
        Self {
            repo_path: repo_path.into(),
            path: path.into(),
            commitish: commitish.into(),
            branch: None,
            auth: GitAuth::default(),
        }
    }

    /// Create the branch `name` at `commitish` and check it out in the
    /// worktree, instead of a detached `HEAD`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_git::cli::CliWorktreeAdd;
    ///
    /// let op = CliWorktreeAdd::new("/tmp/repo", "/tmp/wt", "main").branch("feature");
    /// ```
    pub fn branch(mut self, name: impl Into<String>) -> Self {
        self.branch = Some(name.into());
        self
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Secret`] if the secret store fails, and
    /// [`OperationError::External`] if `commitish` or the branch name is
    /// invalid, git cannot be spawned, or the worktree cannot be added. The
    /// token never appears in the message.
    pub async fn run(
        &self,
        ctx: &OperationContext,
    ) -> Result<CliWorktreeAddOutput, OperationError> {
        if self.commitish.is_empty() || self.commitish.starts_with('-') {
            let message = format!("invalid commit-ish {:?}", self.commitish);
            return Err(external_error(message));
        }
        if let Some(branch) = &self.branch {
            validate_ref_name("branch", branch).await?;
        }
        // `git -C <repo>` would resolve a relative worktree path from the
        // repository instead of the current directory.
        let path = match absolute(&self.path) {
            Ok(path) => path,
            Err(e) => {
                let message = format!("invalid worktree path {}: {e}", self.path.display());
                return Err(external_error(message));
            }
        };
        let _guard = lock_repo(&self.repo_path).await;
        let creds = self.auth.resolve(ctx).await?;

        let mut cmd = git_command(creds.as_ref(), Some(&self.repo_path));
        cmd.args(["worktree", "add", "--quiet"]);
        match &self.branch {
            Some(branch) => cmd.args(["-b", branch.as_str()]),
            None => cmd.arg("--detach"),
        };
        cmd.arg("--").arg(&path).arg(&self.commitish);
        run_git(cmd, "worktree add", creds.as_ref()).await?;

        let mut cmd = git_command(None, Some(&path));
        cmd.args(["rev-parse", "HEAD"]);
        let commit = run_git(cmd, "rev-parse", None).await?;
        Ok(CliWorktreeAddOutput {
            repo_path: self.repo_path.clone(),
            path,
            commit,
            branch: self.branch.clone(),
        })
    }
}

#[async_trait]
impl Operation for CliWorktreeAdd {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(json!({
            "repo_path": self.repo_path,
            "path": self.path,
            "commitish": self.commitish,
            "branch": self.branch,
        }))
    }
}

impl TypedOperation for CliWorktreeAdd {
    type Output = CliWorktreeAddOutput;
}

auth_builders!(CliWorktreeAdd, "cli", "\"/repo\", \"/wt\", \"main\"");
token_builder!(CliWorktreeAdd, "cli", "\"/repo\", \"/wt\", \"main\"");
