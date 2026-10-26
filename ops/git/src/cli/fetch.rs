//! Fetch with the git CLI, partial clones included.

use std::path::PathBuf;

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde_json::{Value, json};

use crate::cli::command::{git_command, run_git, validate_remote_name};
use crate::fetch::FetchPushOutput;
use crate::helpers::{GitAuth, auth_builders, redact_url, to_value, token_builder};
use crate::repo_lock::lock_repo;

/// Fetch refspecs from a remote with the git CLI.
///
/// Works on a partial clone made by [`CliClone`](crate::cli::CliClone),
/// which libgit2 cannot open: git applies the clone filter to the fetch by
/// itself.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::cli::CliFetch;
/// use ironflow_core::operation::{OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// # let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// CliFetch::new("/tmp/repo", "origin", vec!["+refs/heads/*:refs/remotes/origin/*"])
///     .run(&ctx)
///     .await?;
/// # Ok(())
/// # }
/// ```
pub struct CliFetch {
    repo_path: PathBuf,
    remote: String,
    refspecs: Vec<String>,
    auth: GitAuth,
}

impl CliFetch {
    /// Fetch `refspecs` from `remote` into the repository at `repo_path`.
    /// An empty `refspecs` fetches the remote's configured refspecs.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_git::cli::CliFetch;
    /// use ironflow_core::operation::Operation;
    ///
    /// let op = CliFetch::new("/tmp/repo", "origin", vec!["main"]);
    /// assert_eq!(op.kind(), "git");
    /// ```
    pub fn new(
        repo_path: impl Into<PathBuf>,
        remote: impl Into<String>,
        refspecs: Vec<impl Into<String>>,
    ) -> Self {
        Self {
            repo_path: repo_path.into(),
            remote: remote.into(),
            refspecs: refspecs.into_iter().map(Into::into).collect(),
            auth: GitAuth::default(),
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Secret`] if the secret store fails, and
    /// [`OperationError::External`] if the remote name is invalid, git cannot
    /// be spawned or the fetch fails. The token never appears in the message.
    pub async fn run(&self, ctx: &OperationContext) -> Result<FetchPushOutput, OperationError> {
        validate_remote_name(&self.remote)?;
        let _guard = lock_repo(&self.repo_path).await;
        let creds = self.auth.resolve(ctx).await?;
        let mut cmd = git_command(creds.as_ref(), Some(&self.repo_path));
        cmd.args(["fetch", "--quiet", "--", self.remote.as_str()]);
        cmd.args(&self.refspecs);
        run_git(cmd, "fetch", creds.as_ref()).await?;
        Ok(FetchPushOutput {
            remote: redact_url(&self.remote),
            refspecs: self.refspecs.clone(),
        })
    }
}

#[async_trait]
impl Operation for CliFetch {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(json!({
            "repo_path": self.repo_path,
            "remote": redact_url(&self.remote),
            "refspecs": self.refspecs,
        }))
    }
}

impl TypedOperation for CliFetch {
    type Output = FetchPushOutput;
}

auth_builders!(CliFetch, "cli", "\"/repo\", \"origin\", vec![\"main\"]");
token_builder!(CliFetch, "cli", "\"/repo\", \"origin\", vec![\"main\"]");
