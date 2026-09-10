# ironflow-ops-git

Git operations for [Ironflow](https://gitlab.com/ThomasTartrau/ironflow) workflows, powered by the [`git2`](https://crates.io/crates/git2) crate.

Provides Git operations (clone, commit, branch, merge, diff, rebase, stash, tag, submodule, worktree) as tracked workflow steps. All `git2` calls run inside `spawn_blocking` since the library is synchronous.

## Usage

```toml
[dependencies]
ironflow-ops-git = "0.1"
```

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
| `worktree` | Add, remove, list |

## Authentication

No secret store integration. Git credentials (SSH keys, HTTP tokens) are resolved through the standard `git2` credential callback chain (SSH agent, `.gitconfig`, environment variables).
