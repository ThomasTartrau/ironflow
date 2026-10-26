//! Conclude a conflicting merge with the git CLI, partial clones included.

use std::io::{Error as IoError, ErrorKind};
use std::path::{Component, Path, PathBuf};

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::fs::{canonicalize, read, symlink_metadata};
use tokio::process::Command;

use crate::cli::command::{git_command, run_git};
use crate::helpers::{
    GitAuth, GitCredentials, auth_builders, external_error, to_value, token_builder,
};
use crate::repo_lock::lock_repo;

/// Build a `git` command for the merge flow: no repository hook runs
/// (`core.hooksPath=/dev/null`, which `--no-verify` alone does not
/// guarantee), and no editor ever opens.
fn merge_command(creds: Option<&GitCredentials>, repo: &Path) -> Command {
    let mut cmd = git_command(creds, Some(repo));
    cmd.args(["-c", "core.hooksPath=/dev/null"]);
    cmd.env("GIT_EDITOR", "true")
        .env("GIT_MERGE_AUTOEDIT", "no");
    cmd
}

/// Whether `repo` is in the middle of a merge (`MERGE_HEAD` exists).
async fn merge_in_progress(repo: &Path) -> Result<bool, OperationError> {
    let mut cmd = git_command(None, Some(repo));
    cmd.args(["rev-parse", "-q", "--verify", "MERGE_HEAD"]);
    let output = cmd
        .output()
        .await
        .map_err(|e| external_error(format!("failed to spawn git: {e}")))?;
    match output.status.code() {
        Some(0) => Ok(true),
        // `rev-parse -q --verify` exits 1 when the ref is missing; any other
        // code (128: not a repository) is a real git failure.
        Some(1) => Ok(false),
        _ => Err(external_error(format!(
            "git rev-parse failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))),
    }
}

/// Sorted paths of the unmerged files of `repo`, relative to its root.
async fn conflicted_files(repo: &Path) -> Result<Vec<String>, OperationError> {
    let mut cmd = git_command(None, Some(repo));
    cmd.args(["diff", "--name-only", "--diff-filter=U", "-z"]);
    // Not `run_git`: it trims stdout, which would eat the leading whitespace
    // of the first path.
    let output = cmd
        .output()
        .await
        .map_err(|e| external_error(format!("failed to spawn git: {e}")))?;
    if !output.status.success() {
        return Err(external_error(format!(
            "git diff failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut files: Vec<String> = stdout
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(str::to_string)
        .collect();
    files.sort();
    Ok(files)
}

/// Reject a revision git could read as an option.
fn validate_rev(rev: &str) -> Result<(), OperationError> {
    if rev.is_empty() || rev.starts_with('-') || rev.chars().any(char::is_control) {
        return Err(external_error(format!("invalid revision {rev:?}")));
    }
    Ok(())
}

/// Reject a path that is not strictly inside the repository.
fn validate_repo_path(path: &str) -> Result<(), OperationError> {
    let invalid = || external_error(format!("invalid path {path:?}"));
    if path.is_empty() || path.chars().any(char::is_control) {
        return Err(invalid());
    }
    let escapes = Path::new(path).components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    });
    if escapes {
        return Err(invalid());
    }
    Ok(())
}

/// Reject an author identity that is empty, malformed or holds a control
/// character.
fn validate_identity(name: &str, email: &str) -> Result<(), OperationError> {
    let valid = !name.trim().is_empty()
        && !email.trim().is_empty()
        && email.contains('@')
        && !name.chars().any(char::is_control)
        && !email.chars().any(char::is_control);
    if !valid {
        return Err(external_error(format!(
            "invalid author identity {name:?} <{email}>"
        )));
    }
    Ok(())
}

/// Whether `content` holds a conflict marker line (`<<<<<<<` or `>>>>>>>`).
///
/// A lone `=======` is not a marker: it is valid markdown and rst.
fn has_conflict_markers(content: &[u8]) -> bool {
    content.split(|&b| b == b'\n').any(|line| {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        [&b"<<<<<<<"[..], &b">>>>>>>"[..]].iter().any(|marker| {
            line == *marker || (line.starts_with(marker) && line.get(marker.len()) == Some(&b' '))
        })
    })
}

/// Output of [`CliMerge`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliMergeOutput {
    /// Revision that was merged.
    pub revision: String,
    /// Whether the merge stopped on conflicts. The repository then stays in
    /// the merge state.
    pub has_conflicts: bool,
    /// Unmerged files, relative to the repository root, sorted.
    pub conflicted_files: Vec<String>,
    /// Whether `HEAD` already contained the revision: nothing to merge.
    pub up_to_date: bool,
}

/// Merge a revision without committing, and report the conflicted files.
///
/// Runs `git merge --no-commit --no-ff`: a clean merge is staged but not
/// committed, a conflicting one leaves the repository in the merge state
/// with the unmerged files listed in the output. Conclude it with
/// [`CliMergeCommit`] (after resolving the files, checking them with
/// [`CliConflictMarkers`] and staging them with [`CliAdd`]) or undo it with
/// [`CliMergeAbort`].
///
/// Works on a partial clone, which libgit2 cannot open. The merge fetches
/// the missing blobs from the remote, so the token is used for that lazy
/// fetch only; a full clone never touches the network. `repo_path` is the
/// root of the work tree.
///
/// A failure that leaves no unmerged file (dirty tree, unknown revision,
/// unrelated histories, rejected token) is an error, never a conflict.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::cli::CliMerge;
/// use ironflow_core::operation::{OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// # let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let merge = CliMerge::new("/tmp/repo", "origin/main").run(&ctx).await?;
/// if merge.has_conflicts {
///     println!("to resolve: {:?}", merge.conflicted_files);
/// }
/// # Ok(())
/// # }
/// ```
pub struct CliMerge {
    repo_path: PathBuf,
    revision: String,
    auth: GitAuth,
}

impl CliMerge {
    /// Merge `revision` (a branch, tag, SHA or any revision git accepts)
    /// into the checked out branch of the repository at `repo_path`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_git::cli::CliMerge;
    /// use ironflow_core::operation::Operation;
    ///
    /// let op = CliMerge::new("/tmp/repo", "origin/main");
    /// assert_eq!(op.kind(), "git");
    /// ```
    pub fn new(repo_path: impl Into<PathBuf>, revision: impl Into<String>) -> Self {
        Self {
            repo_path: repo_path.into(),
            revision: revision.into(),
            auth: GitAuth::default(),
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Secret`] if the secret store fails, and
    /// [`OperationError::External`] if the revision is invalid, a merge is
    /// already in progress, git cannot be spawned, or the merge fails
    /// without conflicts. The token never appears in the message.
    pub async fn run(&self, ctx: &OperationContext) -> Result<CliMergeOutput, OperationError> {
        validate_rev(&self.revision)?;
        let _guard = lock_repo(&self.repo_path).await;
        let creds = self.auth.resolve(ctx).await?;
        if merge_in_progress(&self.repo_path).await? {
            return Err(external_error("a merge is already in progress"));
        }

        let mut cmd = merge_command(creds.as_ref(), &self.repo_path);
        cmd.args([
            "merge",
            "--no-commit",
            "--no-ff",
            "--no-edit",
            "--no-verify",
        ]);
        cmd.arg("--").arg(&self.revision);
        match run_git(cmd, "merge", creds.as_ref()).await {
            Ok(_) => {
                let staged = merge_in_progress(&self.repo_path).await?;
                Ok(CliMergeOutput {
                    revision: self.revision.clone(),
                    has_conflicts: false,
                    conflicted_files: Vec::new(),
                    up_to_date: !staged,
                })
            }
            Err(err) => {
                let files = conflicted_files(&self.repo_path).await?;
                // Unmerged entries left by a stash pop or an interrupted
                // cherry-pick make git refuse before any merge starts.
                if files.is_empty() || !merge_in_progress(&self.repo_path).await? {
                    return Err(err);
                }
                Ok(CliMergeOutput {
                    revision: self.revision.clone(),
                    has_conflicts: true,
                    conflicted_files: files,
                    up_to_date: false,
                })
            }
        }
    }
}

#[async_trait]
impl Operation for CliMerge {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(json!({
            "repo_path": self.repo_path,
            "revision": self.revision,
        }))
    }
}

impl TypedOperation for CliMerge {
    type Output = CliMergeOutput;
}

auth_builders!(CliMerge, "cli", "\"/repo\", \"origin/main\"");
token_builder!(CliMerge, "cli", "\"/repo\", \"origin/main\"");

/// Output of [`CliConflictMarkers`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliConflictMarkersOutput {
    /// Files that still hold a conflict marker, in the order given.
    pub files_with_markers: Vec<String>,
}

/// Find the files that still hold conflict markers.
///
/// Reads the files of `files` (relative to the root of the work tree) and
/// looks for lines starting with `<<<<<<< ` or `>>>>>>> `. A lone `=======`
/// is not a marker. A missing file (resolved by deleting it) and a symlink
/// (never followed) are clean. It reads files only: no git process, no lock.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::cli::CliConflictMarkers;
/// use ironflow_core::operation::{OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// # let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let out = CliConflictMarkers::new("/tmp/repo", vec!["src/main.rs"])
///     .run(&ctx)
///     .await?;
/// assert!(out.files_with_markers.is_empty());
/// # Ok(())
/// # }
/// ```
pub struct CliConflictMarkers {
    repo_path: PathBuf,
    files: Vec<String>,
}

impl CliConflictMarkers {
    /// Scan `files` of the work tree at `repo_path`, typically
    /// [`CliMergeOutput::conflicted_files`].
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_git::cli::CliConflictMarkers;
    /// use ironflow_core::operation::Operation;
    ///
    /// let op = CliConflictMarkers::new("/tmp/repo", vec!["a.txt"]);
    /// assert_eq!(op.kind(), "git");
    /// ```
    pub fn new(
        repo_path: impl Into<PathBuf>,
        files: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            repo_path: repo_path.into(),
            files: files.into_iter().map(Into::into).collect(),
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] if a path is absolute or leaves
    /// the repository, or if a file cannot be read.
    pub async fn run(
        &self,
        _ctx: &OperationContext,
    ) -> Result<CliConflictMarkersOutput, OperationError> {
        for file in &self.files {
            validate_repo_path(file)?;
        }
        let root = canonicalize(&self.repo_path)
            .await
            .map_err(|e| external_error(format!("invalid repository {:?}: {e}", self.repo_path)))?;
        let mut files_with_markers = Vec::new();
        for file in &self.files {
            let path = self.repo_path.join(file);
            let io_error = |e: IoError| external_error(format!("failed to read {file:?}: {e}"));
            let metadata = match symlink_metadata(&path).await {
                Ok(metadata) => metadata,
                Err(e) if e.kind() == ErrorKind::NotFound => continue,
                Err(e) => return Err(io_error(e)),
            };
            if metadata.file_type().is_symlink() {
                continue;
            }
            // A symlinked directory component would lead the read outside
            // the work tree: the real path must stay under the repository.
            let real = canonicalize(&path).await.map_err(io_error)?;
            if !real.starts_with(&root) {
                return Err(external_error(format!(
                    "path {file:?} leaves the repository"
                )));
            }
            let content = read(&path).await.map_err(io_error)?;
            if has_conflict_markers(&content) {
                files_with_markers.push(file.clone());
            }
        }
        Ok(CliConflictMarkersOutput { files_with_markers })
    }
}

#[async_trait]
impl Operation for CliConflictMarkers {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(json!({
            "repo_path": self.repo_path,
            "files": self.files,
        }))
    }
}

impl TypedOperation for CliConflictMarkers {
    type Output = CliConflictMarkersOutput;
}

/// Output of [`CliAdd`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliAddOutput {
    /// Files that were staged.
    pub staged: Vec<String>,
}

/// Stage files, which marks their conflicts as resolved.
///
/// Runs `git add -A` on the given paths, taken literally (no glob, no
/// pathspec magic): a file deleted to resolve a conflict is staged as a
/// deletion. It is not limited to merges, and it does not check for
/// conflict markers: run [`CliConflictMarkers`] first. Paths are relative to
/// the root of the work tree.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::cli::CliAdd;
/// use ironflow_core::operation::{OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// # let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// CliAdd::new("/tmp/repo", vec!["a.txt"]).run(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct CliAdd {
    repo_path: PathBuf,
    files: Vec<String>,
}

impl CliAdd {
    /// Stage `files` of the work tree at `repo_path`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_git::cli::CliAdd;
    /// use ironflow_core::operation::Operation;
    ///
    /// let op = CliAdd::new("/tmp/repo", vec!["a.txt"]);
    /// assert_eq!(op.kind(), "git");
    /// ```
    pub fn new(
        repo_path: impl Into<PathBuf>,
        files: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            repo_path: repo_path.into(),
            files: files.into_iter().map(Into::into).collect(),
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] if the list is empty, a path is
    /// absolute or leaves the repository, git cannot be spawned, or the
    /// files cannot be staged.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<CliAddOutput, OperationError> {
        if self.files.is_empty() {
            return Err(external_error("no file to add"));
        }
        for file in &self.files {
            validate_repo_path(file)?;
        }
        let _guard = lock_repo(&self.repo_path).await;
        let mut cmd = merge_command(None, &self.repo_path);
        cmd.env("GIT_LITERAL_PATHSPECS", "1");
        cmd.args(["add", "-A", "--"]).args(&self.files);
        run_git(cmd, "add", None).await?;
        Ok(CliAddOutput {
            staged: self.files.clone(),
        })
    }
}

#[async_trait]
impl Operation for CliAdd {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(json!({
            "repo_path": self.repo_path,
            "files": self.files,
        }))
    }
}

impl TypedOperation for CliAdd {
    type Output = CliAddOutput;
}

/// Output of [`CliMergeCommit`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliMergeCommitOutput {
    /// SHA of the merge commit.
    pub commit: String,
}

/// Conclude a merge in progress with a commit.
///
/// Refuses while unmerged files remain: resolve them, check them with
/// [`CliConflictMarkers`] and stage them with [`CliAdd`] first. The author
/// and the committer are the given identity, never read from the
/// configuration, so it works with no `user.*` set. No editor opens, no
/// repository hook runs and the commit is not signed. It never touches the
/// network. `repo_path` is the root of the work tree.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::cli::CliMergeCommit;
/// use ironflow_core::operation::{OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// # let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let out = CliMergeCommit::new("/tmp/repo", "Bot", "bot@example.com")
///     .run(&ctx)
///     .await?;
/// println!("merged in {}", out.commit);
/// # Ok(())
/// # }
/// ```
pub struct CliMergeCommit {
    repo_path: PathBuf,
    author_name: String,
    author_email: String,
    message: Option<String>,
}

impl CliMergeCommit {
    /// Commit the merge of the repository at `repo_path` as
    /// `author_name <author_email>`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_git::cli::CliMergeCommit;
    /// use ironflow_core::operation::Operation;
    ///
    /// let op = CliMergeCommit::new("/tmp/repo", "Bot", "bot@example.com");
    /// assert_eq!(op.kind(), "git");
    /// ```
    pub fn new(
        repo_path: impl Into<PathBuf>,
        author_name: impl Into<String>,
        author_email: impl Into<String>,
    ) -> Self {
        Self {
            repo_path: repo_path.into(),
            author_name: author_name.into(),
            author_email: author_email.into(),
            message: None,
        }
    }

    /// Use `message` instead of the message git prepared for the merge.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_git::cli::CliMergeCommit;
    ///
    /// let op = CliMergeCommit::new("/tmp/repo", "Bot", "bot@example.com")
    ///     .message("Merge main into feature");
    /// ```
    pub fn message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] if the identity is invalid, no
    /// merge is in progress, unmerged files remain (the message names them),
    /// git cannot be spawned, or the commit fails.
    pub async fn run(
        &self,
        _ctx: &OperationContext,
    ) -> Result<CliMergeCommitOutput, OperationError> {
        validate_identity(&self.author_name, &self.author_email)?;
        let _guard = lock_repo(&self.repo_path).await;
        if !merge_in_progress(&self.repo_path).await? {
            return Err(external_error("no merge in progress"));
        }
        let files = conflicted_files(&self.repo_path).await?;
        if !files.is_empty() {
            return Err(external_error(format!(
                "unresolved conflicts: {} (stage the resolved files with CliAdd)",
                files.join(", ")
            )));
        }

        let mut cmd = merge_command(None, &self.repo_path);
        cmd.args(["-c", "commit.gpgsign=false"]);
        cmd.args(["commit", "--no-edit", "--no-verify"]);
        if let Some(message) = &self.message {
            cmd.arg("-m").arg(message);
        }
        cmd.env("GIT_AUTHOR_NAME", &self.author_name)
            .env("GIT_AUTHOR_EMAIL", &self.author_email)
            .env("GIT_COMMITTER_NAME", &self.author_name)
            .env("GIT_COMMITTER_EMAIL", &self.author_email);
        run_git(cmd, "commit", None).await?;

        let mut cmd = git_command(None, Some(&self.repo_path));
        cmd.args(["rev-parse", "HEAD"]);
        let commit = run_git(cmd, "rev-parse", None).await?;
        Ok(CliMergeCommitOutput { commit })
    }
}

#[async_trait]
impl Operation for CliMergeCommit {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(json!({
            "repo_path": self.repo_path,
            "author_name": self.author_name,
            "author_email": self.author_email,
            "message": self.message,
        }))
    }
}

impl TypedOperation for CliMergeCommit {
    type Output = CliMergeCommitOutput;
}

/// Output of [`CliMergeAbort`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliMergeAbortOutput {
    /// Always `true`: the merge was undone.
    pub aborted: bool,
}

/// Abort the merge in progress.
///
/// Runs `git merge --abort`: `HEAD`, the index and the work tree are back to
/// what they were before [`CliMerge`], and `MERGE_HEAD` is gone. On a
/// partial clone, the token is used for the lazy fetch of the blobs the
/// reset needs; a full clone never touches the network.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::cli::CliMergeAbort;
/// use ironflow_core::operation::{OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// # let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// CliMergeAbort::new("/tmp/repo").run(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct CliMergeAbort {
    repo_path: PathBuf,
    auth: GitAuth,
}

impl CliMergeAbort {
    /// Abort the merge of the repository at `repo_path`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_git::cli::CliMergeAbort;
    /// use ironflow_core::operation::Operation;
    ///
    /// let op = CliMergeAbort::new("/tmp/repo");
    /// assert_eq!(op.kind(), "git");
    /// ```
    pub fn new(repo_path: impl Into<PathBuf>) -> Self {
        Self {
            repo_path: repo_path.into(),
            auth: GitAuth::default(),
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Secret`] if the secret store fails, and
    /// [`OperationError::External`] if no merge is in progress, git cannot
    /// be spawned, or the abort fails. The token never appears in the
    /// message.
    pub async fn run(&self, ctx: &OperationContext) -> Result<CliMergeAbortOutput, OperationError> {
        let _guard = lock_repo(&self.repo_path).await;
        let creds = self.auth.resolve(ctx).await?;
        if !merge_in_progress(&self.repo_path).await? {
            return Err(external_error("no merge in progress"));
        }
        let mut cmd = merge_command(creds.as_ref(), &self.repo_path);
        cmd.args(["merge", "--abort"]);
        run_git(cmd, "merge --abort", creds.as_ref()).await?;
        Ok(CliMergeAbortOutput { aborted: true })
    }
}

#[async_trait]
impl Operation for CliMergeAbort {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(json!({ "repo_path": self.repo_path }))
    }
}

impl TypedOperation for CliMergeAbort {
    type Output = CliMergeAbortOutput;
}

auth_builders!(CliMergeAbort, "cli", "\"/repo\"");
token_builder!(CliMergeAbort, "cli", "\"/repo\"");

#[cfg(test)]
mod tests {
    use serde::de::DeserializeOwned;
    use serde_json::from_value;

    use super::*;

    fn round_trip<T: Serialize + DeserializeOwned>(value: &T) -> T {
        from_value(to_value(value).unwrap()).unwrap()
    }

    #[test]
    fn has_conflict_markers_detects_markers() {
        assert!(has_conflict_markers(b"<<<<<<< HEAD\nours\n"));
        assert!(has_conflict_markers(b"a\n=======\nb\n>>>>>>> feature\n"));
        assert!(has_conflict_markers(b"a\n>>>>>>> feature"));
        assert!(has_conflict_markers(b"x\n<<<<<<<\ny\n"));
        assert!(has_conflict_markers(b"x\n>>>>>>>"));
    }

    #[test]
    fn has_conflict_markers_handles_crlf() {
        assert!(has_conflict_markers(b"a\r\n<<<<<<<\r\nb\r\n"));
        assert!(has_conflict_markers(b"a\r\n<<<<<<< HEAD\r\nb\r\n"));
    }

    #[test]
    fn has_conflict_markers_ignores_bare_separator() {
        assert!(!has_conflict_markers(b"Title\n=======\ntext\n"));
        assert!(!has_conflict_markers(b"<<<<<<<<\n>>>>>>>>\n"));
    }

    #[test]
    fn has_conflict_markers_empty_and_binary_input() {
        assert!(!has_conflict_markers(b""));
        assert!(!has_conflict_markers(&[0xff, 0xfe, 0x00, b'\n', 0x80]));
        assert!(has_conflict_markers(b"\xff\n<<<<<<<"));
    }

    #[test]
    fn has_conflict_markers_requires_line_start() {
        assert!(!has_conflict_markers(b"  <<<<<<< HEAD\n"));
        assert!(!has_conflict_markers(b"a <<<<<<< b\n"));
        assert!(!has_conflict_markers(b"x>>>>>>> y\n"));
    }

    #[test]
    fn validate_rev_rejects_empty_dash_and_control_chars() {
        assert!(validate_rev("").is_err());
        assert!(validate_rev("--abort").is_err());
        assert!(validate_rev("-x").is_err());
        assert!(validate_rev("a\nb").is_err());
        assert!(validate_rev("origin/main").is_ok());
        assert!(validate_rev("0123abcd").is_ok());
    }

    #[test]
    fn validate_repo_path_rejects_absolute_and_parent() {
        assert!(validate_repo_path("").is_err());
        assert!(validate_repo_path("/etc/passwd").is_err());
        assert!(validate_repo_path("../x").is_err());
        assert!(validate_repo_path("a/../../x").is_err());
        assert!(validate_repo_path("a\nb").is_err());
        assert!(validate_repo_path("dir/a.txt").is_ok());
        assert!(validate_repo_path("dossier é/fichier.txt").is_ok());
    }

    #[test]
    fn validate_identity_accepts_a_regular_identity() {
        assert!(validate_identity("Bot", "bot@example.com").is_ok());
    }

    #[test]
    fn validate_identity_rejects_empty_control_and_missing_at() {
        assert!(validate_identity("", "bot@example.com").is_err());
        assert!(validate_identity("  ", "bot@example.com").is_err());
        assert!(validate_identity("Bot", "").is_err());
        assert!(validate_identity("Bo\nt", "bot@example.com").is_err());
        assert!(validate_identity("Bot", "bot@exa\nmple.com").is_err());
        assert!(validate_identity("Bot", "bot.example.com").is_err());
    }

    #[test]
    fn outputs_round_trip_through_serde() {
        let merge = CliMergeOutput {
            revision: "feature".to_string(),
            has_conflicts: true,
            conflicted_files: vec!["a.txt".to_string()],
            up_to_date: false,
        };
        let back: CliMergeOutput = round_trip(&merge);
        assert_eq!(back.conflicted_files, merge.conflicted_files);
        assert!(back.has_conflicts && !back.up_to_date);

        let markers = CliConflictMarkersOutput {
            files_with_markers: vec!["a.txt".to_string()],
        };
        let back: CliConflictMarkersOutput = round_trip(&markers);
        assert_eq!(back.files_with_markers, markers.files_with_markers);

        let add = CliAddOutput {
            staged: vec!["a.txt".to_string()],
        };
        let back: CliAddOutput = round_trip(&add);
        assert_eq!(back.staged, add.staged);

        let commit = CliMergeCommitOutput {
            commit: "abc".to_string(),
        };
        let back: CliMergeCommitOutput = round_trip(&commit);
        assert_eq!(back.commit, "abc");

        let abort = CliMergeAbortOutput { aborted: true };
        let back: CliMergeAbortOutput = round_trip(&abort);
        assert!(back.aborted);
    }
}
