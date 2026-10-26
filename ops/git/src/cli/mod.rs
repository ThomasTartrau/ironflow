//! Git operations backed by the `git` binary instead of libgit2.
//!
//! libgit2 cannot clone with a `--filter`, and refuses to open a repository
//! whose configuration holds `extensions.partialclone`: once a repository is
//! a partial clone, [`FetchRemote`](crate::fetch::FetchRemote),
//! [`WorktreeAdd`](crate::worktree::WorktreeAdd) and
//! [`PushRemote`](crate::fetch::PushRemote) can no longer touch it. This
//! module covers that lifecycle with the git CLI:
//!
//! - [`CliClone`]: clone, optionally blobless, bare, without checkout or
//!   with symlinks disabled, published atomically at its target.
//! - [`CliFetch`]: fetch refspecs from a remote.
//! - [`CliWorktreeAdd`]: add a worktree on a new branch or detached.
//! - [`CliPush`]: push a branch, only if every push URL of the remote is the
//!   expected one, in simple, force or force-with-lease mode.
//! - [`CliMerge`]: merge a revision without committing and list the
//!   conflicted files.
//! - [`CliConflictMarkers`]: find the files that still hold conflict markers.
//! - [`CliAdd`]: stage the resolved files.
//! - [`CliMergeCommit`]: conclude the merge with a commit by a given identity.
//! - [`CliMergeAbort`]: undo the merge.
//!
//! # Requirements
//!
//! A `git` binary, version 2.25 or later, on the `PATH` of the worker.
//!
//! # Authentication
//!
//! Like the libgit2 operations, they authenticate over HTTPS with the token
//! held in the `git_token` secret, sent with the username `oauth2`, both
//! overridable with `.token_secret(key)` and `.username(name)`. `.token(t)`
//! passes the token itself and takes precedence over the secret store.
//! Without a token, git uses the ambient credentials (SSH agent, credential
//! helpers).
//!
//! The token reaches git through the environment of the git process, read by
//! an inline credential helper: it is never an argument (visible in the
//! process list), never written to the clone configuration, and the
//! credential helpers configured on the worker are disabled for the call so
//! none of them stores it. It is scrubbed from error messages, and a URL
//! carrying credentials is refused. Repository hooks (`pre-push`,
//! `post-checkout`, ...) do not run when a token is set, so a hook planted in
//! the repository cannot read the token from its environment.
//!
//! git never prompts: a rejected token fails the operation.
//!
//! [`CliMerge`] and [`CliMergeAbort`] use the token only for the lazy fetch of
//! the blobs of a partial clone; a full clone never touches the network. No
//! operation of the merge flow ever opens an editor, signs a commit, or runs
//! a repository hook (`core.hooksPath=/dev/null`, `GIT_EDITOR=true`,
//! `commit.gpgsign=false`).
//!
//! # Concurrency
//!
//! Clone, fetch, worktree add, push and the merge operations ([`CliMerge`],
//! [`CliAdd`], [`CliMergeCommit`], [`CliMergeAbort`]) on the same local
//! repository run one at a time within a process, as do [`FetchRemote`](crate::fetch::FetchRemote)
//! and [`WorktreeAdd`](crate::worktree::WorktreeAdd). Other processes are not
//! covered.
//!
//! # Examples
//!
//! ```no_run
//! use ironflow_ops_git::cli::{CliClone, CliPush, PushMode};
//! use ironflow_core::operation::{OperationContext, NoopSecretResolver};
//! use std::sync::Arc;
//!
//! # async fn example() -> Result<(), ironflow_core::error::OperationError> {
//! # let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
//! let url = "https://gitlab.com/group/repo.git";
//! CliClone::new(url, "/tmp/repo")
//!     .blobless(true)
//!     .run(&ctx)
//!     .await?;
//! CliPush::new("/tmp/repo", url, "feature")
//!     .mode(PushMode::ForceWithLease { expected: None })
//!     .run(&ctx)
//!     .await?;
//! # Ok(())
//! # }
//! ```
//!
//! A conflicting merge, concluded or undone:
//!
//! ```no_run
//! use ironflow_ops_git::cli::{
//!     CliAdd, CliConflictMarkers, CliMerge, CliMergeAbort, CliMergeCommit,
//! };
//! use ironflow_core::operation::{OperationContext, NoopSecretResolver};
//! use std::sync::Arc;
//!
//! # async fn example() -> Result<(), ironflow_core::error::OperationError> {
//! # let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
//! let merge = CliMerge::new("/tmp/repo", "origin/main").run(&ctx).await?;
//! if merge.has_conflicts {
//!     // ... resolve `merge.conflicted_files` in the work tree ...
//!     let files = merge.conflicted_files;
//!     let left = CliConflictMarkers::new("/tmp/repo", files.clone()).run(&ctx).await?;
//!     if left.files_with_markers.is_empty() {
//!         CliAdd::new("/tmp/repo", files).run(&ctx).await?;
//!         CliMergeCommit::new("/tmp/repo", "Bot", "bot@example.com")
//!             .run(&ctx)
//!             .await?;
//!     } else {
//!         CliMergeAbort::new("/tmp/repo").run(&ctx).await?;
//!     }
//! }
//! # Ok(())
//! # }
//! ```

pub mod clone;
pub(crate) mod command;
pub mod fetch;
pub mod merge;
pub mod push;
pub mod worktree;

pub use clone::{CliClone, CliCloneOutput};
pub use fetch::CliFetch;
pub use merge::{
    CliAdd, CliAddOutput, CliConflictMarkers, CliConflictMarkersOutput, CliMerge, CliMergeAbort,
    CliMergeAbortOutput, CliMergeCommit, CliMergeCommitOutput, CliMergeOutput,
};
pub use push::{CliPush, CliPushOutput, PushMode};
pub use worktree::{CliWorktreeAdd, CliWorktreeAddOutput};
