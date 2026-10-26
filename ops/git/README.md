# ironflow-ops-git

Git operations for [Ironflow](https://gitlab.com/ThomasTartrau/ironflow) workflows, powered by the [`git2`](https://crates.io/crates/git2) crate, plus a `cli` family backed by the `git` binary.

Provides Git operations (clone, commit, branch, merge, diff, rebase, stash, tag, submodule, worktree) as tracked workflow steps. All `git2` calls run inside `spawn_blocking` since the library is synchronous.

## Usage

```toml
[dependencies]
ironflow-ops-git = "0.2"
```

This crate follows `git2` 0.21 with the `ssh` and `https` features enabled, and re-exports it (`pub use git2;`).

### Open a repository

```rust,ignore
use ironflow_ops_git::GitRepo;

let repo = GitRepo::open("/path/to/repo")?;
```

### Tracked operations

```rust,ignore
use ironflow_ops_git::GitRepo;
use ironflow_ops_git::commit::CreateCommit;
use ironflow_ops_git::branch::CreateBranch;

let repo = GitRepo::open(".")?;

let branch = CreateBranch::new(&repo, "feature/deploy");
ctx.operation("create-branch", &branch).await?;

let commit = CreateCommit::new(&repo)
    .message("chore: automated update")
    .author("bot", "bot@example.com");
ctx.operation("commit-changes", &commit).await?;
```

## Available operations

| Module | Operations |
|--------|------------|
| `blame` | Blame |
| `branch` | Create, delete, list, rename |
| `checkout` | Checkout branch, file, commit |
| `cli` | Clone (partial, atomic publication), fetch, worktree add, push to an expected URL (simple, force, force-with-lease), conclude a conflicting merge (merge, conflict markers, add, commit, abort), with the `git` binary |
| `cherrypick` | Cherry-pick |
| `commit` | Create, amend |
| `config` | Get, set, list |
| `diff` | Diff (tree, index, workdir) |
| `fetch` | Fetch |
| `graph` | Ahead/behind, merge base |
| `index` | Add, remove, reset |
| `log` | Log with filters |
| `merge` | Merge |
| `object` | Read blob, tree entries |
| `rebase` | Rebase |
| `reflog` | Reflog entries |
| `refs` | Create, delete, list refs |
| `remote` | Add, remove, list, push |
| `repository` | Init, clone |
| `reset` | Reset (soft, mixed, hard) |
| `stash` | Stash, pop, list, drop |
| `status` | Status |
| `submodule` | Init, update, add, sync |
| `tag` | Create, delete, list |
| `worktree` | Add (optionally detached at a commit), remove, list, prune |

## Authentication

Network operations (`RepoClone`, `FetchRemote`, `PushRemote`, `RemotePrune`, `RemoteDefaultBranch`, `SubmoduleUpdate`) authenticate over HTTPS with the token held in the `git_token` workflow secret, sent with the username `oauth2`. Override either per operation:

```rust,ignore
use ironflow_ops_git::repository::RepoClone;

let clone = RepoClone::new("https://gitlab.com/group/private.git", "/tmp/review.git")
    .bare(true)
    .token_secret("gitlab_token")
    .username("oauth2");
ctx.operation("clone", &clone).await?;
```

Without that secret, credentials come from the SSH agent, the git credential helper, then libgit2's default credential, as before.

The `cli` operations also take the token itself with `.token(token)`, which takes precedence over the secret store. An empty token is an error.

The token never lands in a step record: credentials embedded in a URL are masked as `https://***@host/...` in every `input()`, output and error message, and a token read from the secret store is scrubbed from error messages.

`redact_url` is public, for masking URLs in your own messages:

```rust,ignore
use ironflow_ops_git::redact_url;

assert_eq!(redact_url("https://oauth2:t@gitlab.com/g/r.git"), "https://***@gitlab.com/g/r.git");
```

## Git CLI operations

libgit2 can neither make a partial clone nor open one (it refuses a repository with `extensions.partialclone`). The `cli` module runs the `git` binary for the whole lifecycle of such a repository. It needs `git` 2.25 or later on the `PATH` of the worker.

```rust,ignore
use ironflow_ops_git::cli::{CliClone, CliFetch, CliPush, CliWorktreeAdd, PushMode};

let url = "https://gitlab.com/group/repo.git";

// Blobless clone: commits and trees only, file contents on demand.
let clone = CliClone::new(url, "/work/repo")
    .blobless(true)
    .checkout(false)
    .symlinks(false)       // persisted: later worktrees write links as files
    .reuse_existing(true); // reuse a clone of the same URL already there
ctx.operation("clone", &clone).await?;

let fetch = CliFetch::new("/work/repo", "origin", vec!["+refs/heads/*:refs/remotes/origin/*"]);
ctx.operation("fetch", &fetch).await?;

let wt = CliWorktreeAdd::new("/work/repo", "/work/mr-42", "origin/main").branch("mr-42");
ctx.operation("worktree", &wt).await?;

let push = CliPush::new("/work/repo", url, "mr-42")
    .mode(PushMode::ForceWithLease { expected: None });
ctx.operation("push", &push).await?;
```

- `CliClone` clones into a hidden sibling directory and renames it to the target once complete: a failed or cancelled clone leaves nothing at the target. The target must not exist, or be an empty directory, unless `reuse_existing` is set, which accepts an existing clone only if its origin and its bare, blobless and symlinks settings match the request.
- `CliPush` compares every push URL of the remote (`git remote get-url --push --all`) with the expected URL, after normalization (scheme and host case, default port, trailing `/` and `.git`, credentials). If one of them differs, nothing is pushed. Modes: `Simple` (fast-forward only), `Force`, and `ForceWithLease { expected }`, where `expected` is the SHA the remote branch must still point to (`None`: the remote-tracking branch).

A conflicting merge works on a partial clone too (the token only serves the lazy fetch of the missing blobs), and is concluded without editor, hook or signature:

```rust,ignore
use ironflow_ops_git::cli::{
    CliAdd, CliConflictMarkers, CliConflictMarkersOutput, CliMerge, CliMergeAbort, CliMergeCommit,
    CliMergeOutput,
};

let merge = CliMerge::new("/work/mr-42", "origin/main");
let out = ctx.operation("merge", &merge).await?.json::<CliMergeOutput>()?;
if out.has_conflicts {
    // ... resolve `out.conflicted_files` in the work tree ...
    let files = out.conflicted_files;
    let markers = CliConflictMarkers::new("/work/mr-42", files.clone());
    let left = ctx
        .operation("markers", &markers)
        .await?
        .json::<CliConflictMarkersOutput>()?;
    if left.files_with_markers.is_empty() {
        ctx.operation("add", &CliAdd::new("/work/mr-42", files)).await?;
        let commit = CliMergeCommit::new("/work/mr-42", "Bot", "bot@example.com");
        ctx.operation("commit", &commit).await?;
    } else {
        ctx.operation("abort", &CliMergeAbort::new("/work/mr-42")).await?;
    }
}
```

- `CliMerge` runs `git merge --no-commit --no-ff`: a clean merge is staged and still needs `CliMergeCommit`; a failure without unmerged files (dirty tree, unknown revision) is an error, never a conflict. `CliMergeCommit` refuses while unmerged files remain, so stage the resolved files with `CliAdd` first. `CliConflictMarkers` only reads files; a lone `=======` is not a marker.
- The token reaches git through the environment of the git process, read by an inline credential helper. It is never a command-line argument, never written to the clone configuration, and the credential helpers of the worker are disabled for the call so none of them stores it. Repository hooks do not run when a token is set, so a hook planted in the repository cannot read it. git never prompts, and a URL carrying credentials is refused.
- Clone, fetch, worktree add and push on the same local repository run one at a time within a process (`FetchRemote` and `WorktreeAdd` included). Other processes are not covered.
