//! Clone with the git CLI, partial clones included.

use std::ffi::OsString;
use std::io::{Error as IoError, ErrorKind};
use std::path::{Path, PathBuf, absolute};

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tempfile::Builder;
use tokio::fs::{create_dir_all, read_dir, remove_dir, rename, symlink_metadata};
use tokio::process::Command;

use crate::cli::command::{git_command, run_git};
use crate::helpers::{GitAuth, auth_builders, external_error, redact_url, to_value, token_builder};
use crate::repo_lock::lock_repo;
use crate::url::{normalize_url, validate_remote_url};

/// Output of [`CliClone`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliCloneOutput {
    /// URL cloned from, credentials masked.
    pub url: String,
    /// Path of the clone.
    pub path: PathBuf,
    /// Whether the clone is bare.
    pub bare: bool,
    /// Whether the clone was made without blobs (`--filter=blob:none`).
    pub blobless: bool,
    /// Whether the working tree was checked out.
    pub checkout: bool,
    /// Whether the clone creates symbolic links (`core.symlinks`).
    pub symlinks: bool,
    /// `true` when an existing clone of the same URL was reused instead.
    pub reused: bool,
}

/// Clone a repository with the git CLI.
///
/// Unlike [`RepoClone`](crate::repository::RepoClone), it can make a
/// blobless partial clone, which only downloads file contents when they are
/// checked out. The clone is made in a hidden sibling directory of `path`
/// and renamed to `path` once complete: a failed or cancelled clone never
/// leaves a half-written repository at `path`.
///
/// `path` must not exist, or be an empty directory, unless
/// [`CliClone::reuse_existing`] is set.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::cli::CliClone;
/// use ironflow_core::operation::{OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// # let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let clone = CliClone::new("https://gitlab.com/group/repo.git", "/tmp/repo")
///     .blobless(true)
///     .checkout(false)
///     .symlinks(false)
///     .run(&ctx)
///     .await?;
/// assert!(clone.blobless);
/// # Ok(())
/// # }
/// ```
pub struct CliClone {
    url: String,
    path: PathBuf,
    bare: bool,
    blobless: bool,
    checkout: bool,
    symlinks: bool,
    reuse_existing: bool,
    auth: GitAuth,
}

impl CliClone {
    /// Clone `url` into `path`, with a checkout and symlinks enabled.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_git::cli::CliClone;
    /// use ironflow_core::operation::Operation;
    ///
    /// let op = CliClone::new("https://gitlab.com/group/repo.git", "/tmp/repo");
    /// assert_eq!(op.kind(), "git");
    /// ```
    pub fn new(url: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self {
            url: url.into(),
            path: path.into(),
            bare: false,
            blobless: false,
            checkout: true,
            symlinks: true,
            reuse_existing: false,
            auth: GitAuth::default(),
        }
    }

    /// Make a bare clone (`--bare`), without a working tree.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_git::cli::CliClone;
    ///
    /// let op = CliClone::new("https://gitlab.com/group/repo.git", "/tmp/repo.git").bare(true);
    /// ```
    pub fn bare(mut self, bare: bool) -> Self {
        self.bare = bare;
        self
    }

    /// Make a blobless partial clone (`--filter=blob:none`): commits and
    /// trees are downloaded, file contents only when they are needed.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_git::cli::CliClone;
    ///
    /// let op = CliClone::new("https://gitlab.com/group/repo.git", "/tmp/repo").blobless(true);
    /// ```
    pub fn blobless(mut self, blobless: bool) -> Self {
        self.blobless = blobless;
        self
    }

    /// Check out the default branch after cloning (default `true`).
    /// `false` passes `--no-checkout`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_git::cli::CliClone;
    ///
    /// let op = CliClone::new("https://gitlab.com/group/repo.git", "/tmp/repo").checkout(false);
    /// ```
    pub fn checkout(mut self, checkout: bool) -> Self {
        self.checkout = checkout;
        self
    }

    /// Create symbolic links as such (default `true`). `false` sets
    /// `core.symlinks=false` in the clone, so this checkout and every later
    /// checkout or worktree write a link as a plain file holding its target.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_git::cli::CliClone;
    ///
    /// let op = CliClone::new("https://gitlab.com/group/repo.git", "/tmp/repo").symlinks(false);
    /// ```
    pub fn symlinks(mut self, symlinks: bool) -> Self {
        self.symlinks = symlinks;
        self
    }

    /// Reuse `path` when it already holds a clone whose `origin` is the same
    /// URL, without touching the network. The output then has `reused` set
    /// and reports what the existing clone is. A clone of another URL, or
    /// one whose bare, blobless or symlinks setting differs from the
    /// requested one, is an error.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_git::cli::CliClone;
    ///
    /// let op = CliClone::new("https://gitlab.com/group/repo.git", "/tmp/repo")
    ///     .reuse_existing(true);
    /// ```
    pub fn reuse_existing(mut self, reuse_existing: bool) -> Self {
        self.reuse_existing = reuse_existing;
        self
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Secret`] if the secret store fails, and
    /// [`OperationError::External`] if the URL is refused (credentials,
    /// leading `-`, control character, transport helper), `path` has no file
    /// name or already exists, the existing clone has another origin or
    /// another bare, blobless or symlinks setting, git cannot be spawned or
    /// the clone fails. The token never appears in the message.
    pub async fn run(&self, ctx: &OperationContext) -> Result<CliCloneOutput, OperationError> {
        validate_remote_url(&self.url)?;
        let Some(name) = self.path.file_name() else {
            return Err(external_error(format!(
                "clone target {} has no file name",
                self.path.display()
            )));
        };
        let _guard = lock_repo(&self.path).await;

        let existing = match symlink_metadata(&self.path).await {
            Ok(meta) => Some(meta),
            Err(e) if e.kind() == ErrorKind::NotFound => None,
            Err(e) => return Err(io_error("cannot inspect", &self.path, e)),
        };
        if let Some(meta) = existing {
            if meta.is_dir() && is_empty_dir(&self.path).await? {
                if let Err(e) = remove_dir(&self.path).await {
                    return Err(io_error("cannot remove", &self.path, e));
                }
            } else if self.reuse_existing {
                return self.reuse().await;
            } else {
                let message = format!("{} already exists", self.path.display());
                return Err(external_error(message));
            }
        }

        let parent = parent_dir(&self.path);
        if let Err(e) = create_dir_all(parent).await {
            return Err(io_error("cannot create", parent, e));
        }
        let creds = self.auth.resolve(ctx).await?;

        // A sibling of the target is on the same filesystem, so the final
        // rename is atomic. Dropping `staging`, on error or when this future
        // is cancelled, removes whatever git left in it.
        let mut prefix = OsString::from(".");
        prefix.push(name);
        prefix.push(".ironflow-clone-");
        let staging = match Builder::new().prefix(&prefix).tempdir_in(parent) {
            Ok(staging) => staging,
            Err(e) => return Err(io_error("cannot create a staging directory in", parent, e)),
        };
        let staged = staging.path().join("repo");

        let mut cmd = git_command(creds.as_ref(), None);
        cmd.args(["clone", "--quiet"]);
        if self.bare {
            cmd.arg("--bare");
        }
        if !self.checkout {
            cmd.arg("--no-checkout");
        }
        if self.blobless {
            cmd.arg("--filter=blob:none");
        }
        if !self.symlinks {
            cmd.args(["--config", "core.symlinks=false"]);
        }
        cmd.arg("--").arg(&self.url).arg(&staged);
        run_git(cmd, "clone", creds.as_ref()).await?;

        if let Err(e) = rename(&staged, &self.path).await {
            return Err(io_error("cannot move the clone to", &self.path, e));
        }
        Ok(self.output(false))
    }

    /// Accept the clone already at `path` if its `origin` is `self.url` and
    /// it is bare, blobless and has symlinks enabled exactly as requested.
    ///
    /// The output reports what the existing clone is, its checkout state
    /// included, never only what was requested: a workflow that disables
    /// symlinks for untrusted content must not get a clone that has them.
    async fn reuse(&self) -> Result<CliCloneOutput, OperationError> {
        let origin = self.read_config(&["--get", "remote.origin.url"]).await?;
        if normalize_url(&origin) != normalize_url(&self.url) {
            return Err(self.refuse_reuse(format!(
                "its origin {} is not {}",
                redact_url(&origin),
                redact_url(&self.url)
            )));
        }
        let bare = self.read_bool("core.bare", false).await?;
        let symlinks = self.read_bool("core.symlinks", true).await?;
        let filter = self
            .read_config(&["--default", "", "--get", "remote.origin.partialclonefilter"])
            .await?;
        let blobless = filter == "blob:none";
        for (setting, requested, actual) in [
            ("bare", self.bare, bare),
            ("blobless", self.blobless, blobless),
            ("symlinks", self.symlinks, symlinks),
        ] {
            if requested != actual {
                return Err(self.refuse_reuse(format!(
                    "it has {setting} = {actual}, {requested} was requested"
                )));
            }
        }
        let checkout = !bare && self.has_index().await?;
        Ok(CliCloneOutput {
            url: redact_url(&self.url),
            path: self.path.clone(),
            bare,
            blobless,
            checkout,
            symlinks,
            reused: true,
        })
    }

    /// git command run in the existing clone at `path`.
    fn existing_command(&self) -> Command {
        let mut cmd = git_command(None, Some(&self.path));
        // Stop the repository discovery at `path`, so a directory that is
        // not a clone is not mistaken for the repository around it.
        if let Ok(parent) = absolute(parent_dir(&self.path)) {
            cmd.env("GIT_CEILING_DIRECTORIES", parent);
        }
        cmd
    }

    /// Run `git config --local <args>` in the existing clone.
    async fn read_config(&self, args: &[&str]) -> Result<String, OperationError> {
        let mut cmd = self.existing_command();
        cmd.args(["config", "--local"]).args(args);
        match run_git(cmd, "config", None).await {
            Ok(value) => Ok(value),
            Err(e) => Err(self.refuse_reuse(format!("cannot read its configuration: {e}"))),
        }
    }

    /// Boolean setting `key` of the existing clone, `default` when unset.
    async fn read_bool(&self, key: &str, default: bool) -> Result<bool, OperationError> {
        let default = if default { "true" } else { "false" };
        let value = self
            .read_config(&["--type=bool", "--default", default, "--get", key])
            .await?;
        Ok(value == "true")
    }

    /// Whether the existing clone has an index, which a clone made with
    /// `--no-checkout` lacks until something is checked out.
    async fn has_index(&self) -> Result<bool, OperationError> {
        let mut cmd = self.existing_command();
        cmd.args(["rev-parse", "--git-path", "index"]);
        let index = match run_git(cmd, "rev-parse", None).await {
            Ok(index) => self.path.join(index),
            Err(e) => return Err(self.refuse_reuse(format!("cannot locate its index: {e}"))),
        };
        match symlink_metadata(&index).await {
            Ok(_) => Ok(true),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(false),
            Err(e) => Err(io_error("cannot inspect", &index, e)),
        }
    }

    /// Error refusing to reuse the existing clone, for `reason`.
    fn refuse_reuse(&self, reason: String) -> OperationError {
        external_error(format!(
            "refusing to reuse {}: {reason}",
            self.path.display()
        ))
    }

    fn output(&self, reused: bool) -> CliCloneOutput {
        CliCloneOutput {
            url: redact_url(&self.url),
            path: self.path.clone(),
            bare: self.bare,
            blobless: self.blobless,
            checkout: self.checkout,
            symlinks: self.symlinks,
            reused,
        }
    }
}

/// Directory holding `path`, `.` for a bare file name.
fn parent_dir(path: &Path) -> &Path {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    }
}

/// Error for a filesystem call on `path` that failed with `e`.
fn io_error(what: &str, path: &Path, e: IoError) -> OperationError {
    external_error(format!("{what} {}: {e}", path.display()))
}

/// Whether the directory at `path` has no entry.
async fn is_empty_dir(path: &Path) -> Result<bool, OperationError> {
    let mut entries = match read_dir(path).await {
        Ok(entries) => entries,
        Err(e) => return Err(io_error("cannot read", path, e)),
    };
    match entries.next_entry().await {
        Ok(entry) => Ok(entry.is_none()),
        Err(e) => Err(io_error("cannot read", path, e)),
    }
}

#[async_trait]
impl Operation for CliClone {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(json!({
            "url": redact_url(&self.url),
            "path": self.path,
            "bare": self.bare,
            "blobless": self.blobless,
            "checkout": self.checkout,
            "symlinks": self.symlinks,
            "reuse_existing": self.reuse_existing,
        }))
    }
}

impl TypedOperation for CliClone {
    type Output = CliCloneOutput;
}

auth_builders!(
    CliClone,
    "cli",
    "\"https://gitlab.com/group/repo.git\", \"/tmp/repo\""
);
token_builder!(
    CliClone,
    "cli",
    "\"https://gitlab.com/group/repo.git\", \"/tmp/repo\""
);
