//! Push a branch with the git CLI, only to the expected remote URL.

use std::path::PathBuf;

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::cli::command::{git_command, run_git, validate_ref_name, validate_remote_name};
use crate::helpers::{GitAuth, auth_builders, external_error, redact_url, to_value, token_builder};
use crate::repo_lock::lock_repo;
use crate::url::{normalize_url, validate_remote_url};

/// How [`CliPush`] treats a remote branch that is not an ancestor of the
/// pushed commit.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PushMode {
    /// Fast-forward only: the push fails if the remote branch has commits
    /// the local branch lacks.
    #[default]
    Simple,
    /// Overwrite the remote branch whatever it holds (`--force`).
    Force,
    /// Overwrite the remote branch only if it still points where it is
    /// expected to (`--force-with-lease`), so a concurrent push is never
    /// lost.
    ForceWithLease {
        /// SHA the remote branch must point to (7 to 64 hex characters,
        /// known to the local repository). `None` expects the value of the
        /// remote-tracking branch, as left by the last fetch.
        expected: Option<String>,
    },
}

/// Output of [`CliPush`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliPushOutput {
    /// Name of the remote pushed to.
    pub remote: String,
    /// Expected URL the remote was checked against, credentials masked.
    pub url: String,
    /// Branch pushed.
    pub branch: String,
    /// SHA of the pushed commit.
    pub commit: String,
    /// Mode of the push.
    pub mode: PushMode,
}

/// Push a branch with the git CLI, refusing to push anywhere but the
/// expected URL.
///
/// Before pushing, every push URL of the remote (each `pushurl`, or the
/// `url`s when it has none, with `insteadOf` rewrites applied) is compared
/// with `expected_url`, after normalization (case of the scheme and host,
/// default port, trailing `/` and `.git`, credentials). If one of them
/// differs, nothing is pushed: a remote repointed by a previous step, or by
/// the repository content, cannot redirect the push.
///
/// The local branch `branch` is pushed to the branch of the same name.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::cli::{CliPush, PushMode};
/// use ironflow_core::operation::{OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// # let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let pushed = CliPush::new("/tmp/repo", "https://gitlab.com/group/repo.git", "feature")
///     .mode(PushMode::ForceWithLease { expected: None })
///     .run(&ctx)
///     .await?;
/// println!("pushed {}", pushed.commit);
/// # Ok(())
/// # }
/// ```
pub struct CliPush {
    repo_path: PathBuf,
    expected_url: String,
    branch: String,
    remote: String,
    mode: PushMode,
    auth: GitAuth,
}

impl CliPush {
    /// Push the local `branch` of the repository at `repo_path` to the remote
    /// `origin`, which must point to `expected_url`, in [`PushMode::Simple`].
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_git::cli::CliPush;
    /// use ironflow_core::operation::Operation;
    ///
    /// let op = CliPush::new("/tmp/repo", "https://gitlab.com/group/repo.git", "feature");
    /// assert_eq!(op.kind(), "git");
    /// ```
    pub fn new(
        repo_path: impl Into<PathBuf>,
        expected_url: impl Into<String>,
        branch: impl Into<String>,
    ) -> Self {
        Self {
            repo_path: repo_path.into(),
            expected_url: expected_url.into(),
            branch: branch.into(),
            remote: "origin".to_string(),
            mode: PushMode::default(),
            auth: GitAuth::default(),
        }
    }

    /// Push in `mode` instead of [`PushMode::Simple`].
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_git::cli::{CliPush, PushMode};
    ///
    /// let op = CliPush::new("/tmp/repo", "https://gitlab.com/group/repo.git", "feature")
    ///     .mode(PushMode::Force);
    /// ```
    pub fn mode(mut self, mode: PushMode) -> Self {
        self.mode = mode;
        self
    }

    /// Push to the remote `name` instead of `origin`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_git::cli::CliPush;
    ///
    /// let op = CliPush::new("/tmp/repo", "https://gitlab.com/group/repo.git", "feature")
    ///     .remote("upstream");
    /// ```
    pub fn remote(mut self, name: impl Into<String>) -> Self {
        self.remote = name.into();
        self
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Secret`] if the secret store fails, and
    /// [`OperationError::External`] if `expected_url`, the branch, the remote
    /// name or the lease SHA is invalid, the remote does not exist, has no
    /// push URL or has one that is not `expected_url`, the local branch does
    /// not exist, git cannot be spawned, or the push fails. The token never
    /// appears in the message.
    pub async fn run(&self, ctx: &OperationContext) -> Result<CliPushOutput, OperationError> {
        validate_remote_url(&self.expected_url)?;
        validate_ref_name("branch", &self.branch).await?;
        validate_remote_name(&self.remote)?;
        let lease = match &self.mode {
            PushMode::ForceWithLease { expected } => expected.as_deref(),
            _ => None,
        };
        if let Some(sha) = lease {
            validate_lease_sha(sha)?;
        }
        let _guard = lock_repo(&self.repo_path).await;

        let mut cmd = git_command(None, Some(&self.repo_path));
        cmd.args(["remote", "get-url", "--push", "--all", "--"]);
        cmd.arg(&self.remote);
        let urls = run_git(cmd, "remote get-url", None).await?;
        let urls: Vec<&str> = urls
            .lines()
            .map(str::trim)
            .filter(|url| !url.is_empty())
            .collect();
        check_push_urls(&self.remote, &urls, &self.expected_url)?;

        let local_ref = format!("refs/heads/{}", self.branch);
        let mut cmd = git_command(None, Some(&self.repo_path));
        cmd.args(["rev-parse", "--verify"]);
        cmd.arg(format!("{local_ref}^{{commit}}"));
        let commit = match run_git(cmd, "rev-parse", None).await {
            Ok(commit) => commit,
            Err(e) => {
                let message = format!("local branch {:?} not found: {e}", self.branch);
                return Err(external_error(message));
            }
        };

        let creds = self.auth.resolve(ctx).await?;
        let mut cmd = git_command(creds.as_ref(), Some(&self.repo_path));
        cmd.args(["push", "--quiet"]);
        if let Some(flag) = mode_flag(&self.mode, &self.branch) {
            cmd.arg(flag);
        }
        cmd.args(["--", self.remote.as_str()]);
        cmd.arg(format!("{local_ref}:{local_ref}"));
        run_git(cmd, "push", creds.as_ref()).await?;

        Ok(CliPushOutput {
            remote: self.remote.clone(),
            url: redact_url(&self.expected_url),
            branch: self.branch.clone(),
            commit,
            mode: self.mode.clone(),
        })
    }
}

/// Refuse the push unless every URL of `urls` is `expected`, once
/// normalized. An empty list is refused too.
fn check_push_urls(remote: &str, urls: &[&str], expected: &str) -> Result<(), OperationError> {
    if urls.is_empty() {
        let message = format!("refusing to push: remote `{remote}` has no push URL");
        return Err(external_error(message));
    }
    let expected_normalized = normalize_url(expected);
    for url in urls {
        if normalize_url(url) != expected_normalized {
            return Err(external_error(format!(
                "refusing to push: remote `{remote}` points to {}, expected {}",
                redact_url(url),
                redact_url(expected)
            )));
        }
    }
    Ok(())
}

/// The `git push` flag of `mode` for `branch`, `None` for a simple push.
fn mode_flag(mode: &PushMode, branch: &str) -> Option<String> {
    match mode {
        PushMode::Simple => None,
        PushMode::Force => Some("--force".to_string()),
        PushMode::ForceWithLease { expected } => {
            let lease = format!("--force-with-lease=refs/heads/{branch}");
            Some(match expected {
                Some(sha) => format!("{lease}:{sha}"),
                None => lease,
            })
        }
    }
}

/// Check that `sha` is 7 to 64 hexadecimal characters.
fn validate_lease_sha(sha: &str) -> Result<(), OperationError> {
    if (7..=64).contains(&sha.len()) && sha.chars().all(|c| c.is_ascii_hexdigit()) {
        return Ok(());
    }
    Err(external_error(format!(
        "invalid lease SHA {sha:?}: expected 7 to 64 hexadecimal characters"
    )))
}

#[async_trait]
impl Operation for CliPush {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(json!({
            "repo_path": self.repo_path,
            "expected_url": redact_url(&self.expected_url),
            "branch": self.branch,
            "remote": self.remote,
            "mode": self.mode,
        }))
    }
}

impl TypedOperation for CliPush {
    type Output = CliPushOutput;
}

auth_builders!(CliPush, "cli", "\"/repo\", \"https://h/r.git\", \"main\"");
token_builder!(CliPush, "cli", "\"/repo\", \"https://h/r.git\", \"main\"");

#[cfg(test)]
mod tests {
    use super::*;

    const EXPECTED: &str = "https://gitlab.com/g/r.git";

    #[test]
    fn mode_flag_per_mode() {
        assert_eq!(mode_flag(&PushMode::Simple, "b"), None);
        let force = mode_flag(&PushMode::Force, "b");
        assert_eq!(force.as_deref(), Some("--force"));
        let lease = PushMode::ForceWithLease { expected: None };
        assert_eq!(
            mode_flag(&lease, "b").as_deref(),
            Some("--force-with-lease=refs/heads/b")
        );
        let lease = PushMode::ForceWithLease {
            expected: Some("abc1234".to_string()),
        };
        assert_eq!(
            mode_flag(&lease, "b").as_deref(),
            Some("--force-with-lease=refs/heads/b:abc1234")
        );
    }

    #[test]
    fn lease_sha_must_be_7_to_64_hex_chars() {
        validate_lease_sha("abc1234").unwrap();
        validate_lease_sha(&"a".repeat(40)).unwrap();
        validate_lease_sha(&"0".repeat(64)).unwrap();
        for sha in ["", "abc123", "abc123z", "-abc1234", "HEAD~1234"] {
            assert!(validate_lease_sha(sha).is_err(), "accepted: {sha:?}");
        }
        assert!(validate_lease_sha(&"a".repeat(65)).is_err());
    }

    #[test]
    fn matching_push_urls_are_accepted() {
        let urls = [EXPECTED, "https://GitLab.com/g/r/"];
        check_push_urls("origin", &urls, EXPECTED).unwrap();
    }

    #[test]
    fn a_mismatching_push_url_after_a_match_is_refused() {
        let urls = [EXPECTED, "https://evil.example/g/r.git"];
        let message = check_push_urls("origin", &urls, EXPECTED)
            .unwrap_err()
            .to_string();
        assert!(message.contains("refusing to push"), "got: {message}");
        assert!(message.contains("evil.example"), "got: {message}");
    }

    #[test]
    fn a_mismatching_push_url_before_a_match_is_refused() {
        let urls = ["https://evil.example/g/r.git", EXPECTED];
        check_push_urls("origin", &urls, EXPECTED).unwrap_err();
    }

    #[test]
    fn no_push_url_is_refused() {
        let message = check_push_urls("origin", &[], EXPECTED)
            .unwrap_err()
            .to_string();
        assert!(message.contains("no push URL"), "got: {message}");
    }

    #[test]
    fn mismatching_url_with_credentials_is_redacted() {
        let urls = ["https://oauth2:s3cret@evil.example/g/r.git"];
        let message = check_push_urls("origin", &urls, EXPECTED)
            .unwrap_err()
            .to_string();
        assert!(!message.contains("s3cret"), "got: {message}");
        assert!(message.contains("***@evil.example"), "got: {message}");
    }

    #[test]
    fn push_mode_serializes_in_snake_case() {
        let mode = PushMode::ForceWithLease { expected: None };
        let expected = json!({ "force_with_lease": { "expected": null } });
        assert_eq!(to_value(&mode).unwrap(), expected);
        assert_eq!(to_value(&PushMode::Simple).unwrap(), "simple");
    }
}
