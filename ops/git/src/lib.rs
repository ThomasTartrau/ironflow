//! Git operations for Ironflow workflows, powered by [`git2`].
//!
//! This crate provides a comprehensive set of Git operations as Ironflow
//! [`Operation`](ironflow_core::operation::Operation) implementations. Each
//! operation wraps a [`git2`] API call, running it inside
//! [`spawn_blocking`](tokio::task::spawn_blocking) since `git2` is synchronous.
//!
//! # Architecture
//!
//! - [`GitRepo`] is the central handle, wrapping a repository path
//! - Each operation is a standalone struct implementing [`Operation`](ironflow_core::operation::Operation)
//! - All operations return `kind() == "git"`
//! - Parameters are set at construction time, not via [`OperationContext`](ironflow_core::operation::OperationContext)
//!
//! # Quick start
//!
//! ```no_run
//! use ironflow_ops_git::GitRepo;
//! use ironflow_ops_git::repository::RepoInit;
//! use ironflow_ops_git::commit::CommitCreate;
//! use ironflow_ops_git::index::IndexAdd;
//! use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
//! use std::sync::Arc;
//!
//! # async fn example() -> Result<(), ironflow_core::error::OperationError> {
//! let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
//!
//! // Initialize a repo
//! let init = RepoInit::new("/tmp/my-repo", false);
//! init.execute(&ctx).await?;
//!
//! // Stage a file and commit
//! let add = IndexAdd::new("/tmp/my-repo", "README.md");
//! add.execute(&ctx).await?;
//!
//! let commit = CommitCreate::new("/tmp/my-repo", "Initial commit", "Alice", "alice@example.com");
//! commit.execute(&ctx).await?;
//! # Ok(())
//! # }
//! ```
//!
//! # Tracked operations
//!
//! Every operation implements [`Operation`](ironflow_core::operation::Operation),
//! so it can be passed to `WorkflowContext::operation()` for step lifecycle
//! tracking (step record, status transitions, duration, output persistence).
//!
//! # Modules
//!
//! Operations are organized by Git domain:
//!
//! | Module | Operations |
//! |--------|-----------|
//! | [`repository`] | Init, Open, Clone, Discover, State |
//! | [`index`] | Add, AddAll, Remove, RemoveAll, UpdateAll, WriteTree |
//! | [`commit`] | Create, Find, Amend, Signed |
//! | [`branch`] | Create, Delete, Rename, List, Lookup, IsHead, SetUpstream |
//! | [`tag`] | CreateLightweight, CreateAnnotated, Delete, List, ListMatch |
//! | [`remote`] | Create, Delete, Rename, SetUrl, List, Lookup |
//! | [`fetch`] | Fetch, Push, Prune, DefaultBranch |
//! | [`merge`] | Branch, Analysis, Commits, Base, CleanupState |
//! | [`rebase`] | Init, Next, Commit, Abort, Finish |
//! | [`cherrypick`] | Cherrypick, CherrypickCommit, Revert, RevertCommit |
//! | [`stash`] | Save, Apply, Pop, Drop, List |
//! | [`diff`] | TreeToTree, TreeToIndex, IndexToWorkdir, Stats, FindSimilar, Apply |
//! | [`checkout`] | Head, Index, Tree |
//! | [`blame`] | BlameFile |
//! | [`log`] | RevwalkNew, RevwalkPushRange, RevwalkSimplifyFirstParent |
//! | [`refs`] | Create, Delete, Rename, Lookup, NameToId |
//! | [`reflog`] | Read, Append, Drop |
//! | [`submodule`] | Add, Init, Update, Lookup, List |
//! | [`worktree`] | Add, List, Validate, Prune |
//! | [`config`] | Get, Set, Delete, List |
//! | [`status`] | File, List, ShouldIgnore |
//! | [`reset`] | Reset (soft, mixed, hard) |
//! | [`graph`] | AheadBehind, DescendantOf, Describe |
//! | [`object`] | BlobCreate, TreeLookup, FindObject |

pub mod blame;
pub mod branch;
pub mod checkout;
pub mod cherrypick;
pub mod commit;
pub mod config;
pub mod diff;
pub mod fetch;
pub mod graph;
mod helpers;
pub mod index;
pub mod log;
pub mod merge;
pub mod object;
pub mod rebase;
pub mod reflog;
pub mod refs;
pub mod remote;
mod repo;
pub mod repository;
pub mod reset;
pub mod stash;
pub mod status;
pub mod submodule;
pub mod tag;
pub mod worktree;

pub use git2;
pub use repo::GitRepo;

#[cfg(test)]
pub(crate) mod test_helpers {
    use std::fs;
    use std::path::Path;
    use std::sync::Arc;

    use git2::{Oid, Repository, Signature};
    use ironflow_core::operation::{NoopSecretResolver, OperationContext};

    pub(crate) fn ctx() -> OperationContext {
        OperationContext::new(Arc::new(NoopSecretResolver))
    }

    pub(crate) fn init_repo(path: &Path) -> Oid {
        let repo = Repository::init(path).unwrap();
        fs::write(path.join("file.txt"), "content").unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_path(Path::new("file.txt")).unwrap();
        idx.write().unwrap();
        let tree = repo.find_tree(idx.write_tree().unwrap()).unwrap();
        let sig = Signature::now("Test", "test@test.com").unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap()
    }

    pub(crate) fn make_two_commits(path: &Path) -> (String, String) {
        let repo = Repository::init(path).unwrap();
        let sig = Signature::now("Test", "test@test.com").unwrap();
        fs::write(path.join("file.txt"), "v1").unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_path(Path::new("file.txt")).unwrap();
        idx.write().unwrap();
        let tree = repo.find_tree(idx.write_tree().unwrap()).unwrap();
        let c1 = repo
            .commit(Some("HEAD"), &sig, &sig, "first", &tree, &[])
            .unwrap();
        let parent = repo.find_commit(c1).unwrap();

        fs::write(path.join("other.txt"), "v2").unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_path(Path::new("other.txt")).unwrap();
        idx.write().unwrap();
        let tree2 = repo.find_tree(idx.write_tree().unwrap()).unwrap();
        let c2 = repo
            .commit(Some("HEAD"), &sig, &sig, "second", &tree2, &[&parent])
            .unwrap();
        (c1.to_string(), c2.to_string())
    }
}
