//! Concluding a conflicting merge with the git CLI, on a local repository
//! and on a blobless clone.

mod common;

use std::env::set_var;
use std::fs::{Permissions, create_dir, read_to_string, remove_file, set_permissions, write};
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::Path;
use std::process::Command as StdCommand;
use std::sync::Arc;
use std::time::Duration;

use common::{GitHttpServer, TOKEN, ctx_with_token};
use ironflow_core::operation::{NoopSecretResolver, OperationContext};
use ironflow_ops_git::cli::{
    CliAdd, CliClone, CliConflictMarkers, CliMerge, CliMergeAbort, CliMergeCommit, CliWorktreeAdd,
};
use tempfile::{TempDir, tempdir};
use tokio::time::timeout;

const LIMIT: Duration = Duration::from_secs(30);

/// Resolution written over the conflicted `a.txt`.
const RESOLVED: &str = "resolved\nline 2\n";

fn ctx() -> OperationContext {
    OperationContext::new(Arc::new(NoopSecretResolver))
}

/// Run `git` in `dir` with a fixed identity and return its trimmed stdout.
fn git(dir: &Path, args: &[&str]) -> String {
    let output = StdCommand::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// Point git at an empty global config and an editor that fails, so a
/// missing identity or a launched editor breaks the test.
fn isolate_git_config(home: &Path) {
    let config = home.join("gitconfig");
    write(&config, "").unwrap();
    // SAFETY: std serializes `set_var` with the environment reads of every
    // process spawned from Rust, which is all this test binary does.
    unsafe {
        set_var("HOME", home);
        set_var("GIT_CONFIG_GLOBAL", &config);
        set_var("GIT_CONFIG_NOSYSTEM", "1");
        set_var("GIT_EDITOR", "false");
    }
}

/// Repository on `main` that conflicts with `feature` on `a.txt`.
/// With `clean`, `feature` changes `b.txt` instead: no conflict.
fn repo(clean: bool) -> TempDir {
    let dir = tempdir().unwrap();
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    write(p.join("a.txt"), "base\nline 2\n").unwrap();
    write(p.join("b.txt"), "b base\n").unwrap();
    git(p, &["add", "."]);
    git(p, &["commit", "-q", "-m", "base"]);
    git(p, &["checkout", "-q", "-b", "feature"]);
    if clean {
        write(p.join("b.txt"), "b feature\n").unwrap();
    } else {
        write(p.join("a.txt"), "feature\nline 2\n").unwrap();
    }
    git(p, &["commit", "-q", "-am", "feature"]);
    git(p, &["checkout", "-q", "main"]);
    write(p.join("a.txt"), "main\nline 2\n").unwrap();
    git(p, &["commit", "-q", "-am", "main"]);
    dir
}

fn merge_head(p: &Path) -> bool {
    p.join(".git").join("MERGE_HEAD").exists()
}

fn unmerged(p: &Path) -> String {
    git(p, &["diff", "--diff-filter=U", "--name-only"])
}

/// Conflicted merge of `feature` into `main`.
async fn conflicted() -> TempDir {
    let dir = repo(false);
    let out = CliMerge::new(dir.path(), "feature")
        .run(&ctx())
        .await
        .unwrap();
    assert!(out.has_conflicts);
    dir
}

/// Resolve `a.txt` and stage it.
async fn resolve(p: &Path) {
    write(p.join("a.txt"), RESOLVED).unwrap();
    let markers = CliConflictMarkers::new(p, ["a.txt"])
        .run(&ctx())
        .await
        .unwrap();
    assert!(markers.files_with_markers.is_empty());
    CliAdd::new(p, ["a.txt"]).run(&ctx()).await.unwrap();
}

#[tokio::test]
async fn cli_merge_reports_conflicted_files() {
    timeout(LIMIT, async {
        let dir = repo(false);
        let out = CliMerge::new(dir.path(), "feature")
            .run(&ctx())
            .await
            .unwrap();
        assert!(out.has_conflicts && !out.up_to_date);
        assert_eq!(out.revision, "feature");
        assert_eq!(out.conflicted_files, vec!["a.txt".to_string()]);
        assert!(merge_head(dir.path()));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn cli_merge_clean_merge_is_staged_not_committed() {
    timeout(LIMIT, async {
        let dir = repo(true);
        let p = dir.path();
        let before = git(p, &["rev-parse", "HEAD"]);
        let out = CliMerge::new(p, "feature").run(&ctx()).await.unwrap();
        assert!(!out.has_conflicts && !out.up_to_date);
        assert!(out.conflicted_files.is_empty());
        assert_eq!(git(p, &["rev-parse", "HEAD"]), before);
        assert!(merge_head(p));
        CliMergeCommit::new(p, "Bot", "bot@example.com")
            .run(&ctx())
            .await
            .unwrap();
        assert_ne!(git(p, &["rev-parse", "HEAD"]), before);
        assert!(!merge_head(p));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn cli_merge_up_to_date() {
    timeout(LIMIT, async {
        let dir = repo(false);
        let out = CliMerge::new(dir.path(), "HEAD~1")
            .run(&ctx())
            .await
            .unwrap();
        assert!(out.up_to_date && !out.has_conflicts);
        assert!(!merge_head(dir.path()));
        let err = CliMergeAbort::new(dir.path())
            .run(&ctx())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no merge in progress"));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn cli_merge_unknown_revision_is_an_error() {
    timeout(LIMIT, async {
        let dir = repo(false);
        CliMerge::new(dir.path(), "nope")
            .run(&ctx())
            .await
            .unwrap_err();
        assert!(!merge_head(dir.path()));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn cli_merge_rejects_option_like_revision() {
    timeout(LIMIT, async {
        let dir = repo(false);
        for rev in ["--abort", "", "a\nb"] {
            CliMerge::new(dir.path(), rev)
                .run(&ctx())
                .await
                .unwrap_err();
        }
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn cli_merge_refuses_when_a_merge_is_in_progress() {
    timeout(LIMIT, async {
        let dir = conflicted().await;
        let err = CliMerge::new(dir.path(), "feature")
            .run(&ctx())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("already in progress"));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn cli_merge_on_dirty_tree_is_an_error_not_a_conflict() {
    timeout(LIMIT, async {
        let dir = repo(true);
        write(dir.path().join("b.txt"), "local edit\n").unwrap();
        CliMerge::new(dir.path(), "feature")
            .run(&ctx())
            .await
            .unwrap_err();
        assert!(!merge_head(dir.path()));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn conflict_markers_found_then_cleared() {
    timeout(LIMIT, async {
        let dir = conflicted().await;
        let p = dir.path();
        let files = ["a.txt", "b.txt"];
        let out = CliConflictMarkers::new(p, files).run(&ctx()).await.unwrap();
        assert_eq!(out.files_with_markers, vec!["a.txt".to_string()]);
        write(p.join("a.txt"), RESOLVED).unwrap();
        let out = CliConflictMarkers::new(p, files).run(&ctx()).await.unwrap();
        assert!(out.files_with_markers.is_empty());
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn conflict_markers_ignore_bare_separator_missing_file_and_symlink() {
    timeout(LIMIT, async {
        let dir = tempdir().unwrap();
        let p = dir.path();
        write(p.join("doc.md"), "Title\n=======\ntext\n").unwrap();
        let outside = tempdir().unwrap();
        write(outside.path().join("x"), "<<<<<<< HEAD\n").unwrap();
        symlink(outside.path().join("x"), p.join("link")).unwrap();
        let files = ["doc.md", "gone.txt", "link"];
        let out = CliConflictMarkers::new(p, files).run(&ctx()).await.unwrap();
        assert!(out.files_with_markers.is_empty());
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn conflict_markers_reject_symlinked_directory_and_missing_repo() {
    timeout(LIMIT, async {
        let dir = tempdir().unwrap();
        let outside = tempdir().unwrap();
        write(outside.path().join("x"), "<<<<<<< HEAD\n").unwrap();
        symlink(outside.path(), dir.path().join("dir")).unwrap();
        let op = CliConflictMarkers::new(dir.path(), ["dir/x"]);
        op.run(&ctx()).await.unwrap_err();

        let missing = dir.path().join("missing");
        let op = CliConflictMarkers::new(&missing, ["a.txt"]);
        op.run(&ctx()).await.unwrap_err();
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn conflict_markers_reject_path_outside_repo() {
    timeout(LIMIT, async {
        let dir = tempdir().unwrap();
        for path in ["../x", "/etc/passwd", ""] {
            let op = CliConflictMarkers::new(dir.path(), [path]);
            op.run(&ctx()).await.unwrap_err();
        }
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn cli_add_stages_resolution_and_clears_unmerged_state() {
    timeout(LIMIT, async {
        let dir = conflicted().await;
        let p = dir.path();
        write(p.join("a.txt"), RESOLVED).unwrap();
        assert_eq!(unmerged(p), "a.txt");
        let out = CliAdd::new(p, ["a.txt"]).run(&ctx()).await.unwrap();
        assert_eq!(out.staged, vec!["a.txt".to_string()]);
        assert_eq!(unmerged(p), "");

        remove_file(p.join("b.txt")).unwrap();
        CliAdd::new(p, ["b.txt"]).run(&ctx()).await.unwrap();
        assert_eq!(
            git(p, &["diff", "--cached", "--name-status", "--", "b.txt"]),
            "D\tb.txt"
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn cli_add_rejects_empty_list_and_paths_outside_repo() {
    timeout(LIMIT, async {
        let dir = repo(false);
        let none: [&str; 0] = [];
        CliAdd::new(dir.path(), none).run(&ctx()).await.unwrap_err();
        for path in ["../x", "/etc/passwd"] {
            CliAdd::new(dir.path(), [path])
                .run(&ctx())
                .await
                .unwrap_err();
        }
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn cli_add_takes_pathspecs_literally() {
    timeout(LIMIT, async {
        let dir = repo(false);
        let p = dir.path();
        write(p.join("*.txt"), "star\n").unwrap();
        write(p.join(":(exclude)a.txt"), "magic\n").unwrap();
        write(p.join("other.txt"), "other\n").unwrap();
        let files = ["*.txt", ":(exclude)a.txt"];
        CliAdd::new(p, files).run(&ctx()).await.unwrap();
        let staged = git(p, &["diff", "--cached", "--name-only"]);
        assert!(staged.contains("*.txt") && staged.contains(":(exclude)a.txt"));
        assert!(!staged.contains("other.txt"));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn merge_commit_concludes_with_given_identity() {
    timeout(LIMIT, async {
        let home = tempdir().unwrap();
        isolate_git_config(home.path());
        let dir = conflicted().await;
        let p = dir.path();
        resolve(p).await;
        // The repository holds no `user.*`: the identity comes from the op.
        let op = CliMergeCommit::new(p, "Bot", "bot@example.com");
        let out = op.run(&ctx()).await.unwrap();
        assert_eq!(out.commit, git(p, &["rev-parse", "HEAD"]));
        let who = git(p, &["log", "-1", "--format=%an,%ae,%cn,%ce"]);
        assert_eq!(who, "Bot,bot@example.com,Bot,bot@example.com");
        let parents = git(p, &["rev-list", "--parents", "-n", "1", "HEAD"]);
        assert_eq!(parents.split(' ').count(), 3);
        assert!(!merge_head(p));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn merge_commit_message_sets_the_subject() {
    timeout(LIMIT, async {
        let dir = conflicted().await;
        let p = dir.path();
        resolve(p).await;
        let op = CliMergeCommit::new(p, "Bot", "bot@example.com").message("custom");
        op.run(&ctx()).await.unwrap();
        assert_eq!(git(p, &["log", "-1", "--format=%s"]), "custom");
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn merge_commit_ignores_repo_hooks_and_signing_config() {
    timeout(LIMIT, async {
        let dir = repo(false);
        let p = dir.path();
        let hooks = p.join(".git").join("hooks");
        let names = [
            "pre-merge-commit",
            "pre-commit",
            "prepare-commit-msg",
            "commit-msg",
            "post-commit",
            "post-merge",
        ];
        for name in names {
            let path = hooks.join(name);
            write(&path, format!("#!/bin/sh\ntouch hook-ran-{name}\nexit 1\n")).unwrap();
            set_permissions(&path, Permissions::from_mode(0o755)).unwrap();
        }
        git(p, &["config", "commit.gpgsign", "true"]);
        git(p, &["config", "gpg.program", "/bin/false"]);

        let out = CliMerge::new(p, "feature").run(&ctx()).await.unwrap();
        assert!(out.has_conflicts);
        resolve(p).await;
        CliMergeCommit::new(p, "Bot", "bot@example.com")
            .run(&ctx())
            .await
            .unwrap();
        assert!(!merge_head(p));
        for name in names {
            assert!(!p.join(format!("hook-ran-{name}")).exists(), "{name} ran");
        }
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn merge_commit_refuses_unresolved_conflicts() {
    timeout(LIMIT, async {
        let dir = conflicted().await;
        let p = dir.path();
        let head = git(p, &["rev-parse", "HEAD"]);
        let op = CliMergeCommit::new(p, "Bot", "bot@example.com");
        let err = op.run(&ctx()).await.unwrap_err();
        assert!(err.to_string().contains("a.txt"));

        // Resolved but not staged: still unmerged.
        write(p.join("a.txt"), RESOLVED).unwrap();
        let err = op.run(&ctx()).await.unwrap_err();
        assert!(err.to_string().contains("a.txt"));
        assert_eq!(git(p, &["rev-parse", "HEAD"]), head);
        assert!(merge_head(p));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn merge_commit_without_merge_in_progress_is_an_error() {
    timeout(LIMIT, async {
        let dir = repo(false);
        let op = CliMergeCommit::new(dir.path(), "Bot", "bot@example.com");
        let err = op.run(&ctx()).await.unwrap_err();
        assert!(err.to_string().contains("no merge in progress"));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn merge_commit_rejects_invalid_identity() {
    timeout(LIMIT, async {
        let dir = conflicted().await;
        resolve(dir.path()).await;
        let identities = [
            ("", "bot@example.com"),
            ("Bo\nt", "bot@example.com"),
            ("Bot", "bot.example.com"),
        ];
        for (name, email) in identities {
            let op = CliMergeCommit::new(dir.path(), name, email);
            op.run(&ctx()).await.unwrap_err();
        }
        assert!(merge_head(dir.path()));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn merge_abort_restores_pre_merge_state() {
    timeout(LIMIT, async {
        let dir = repo(false);
        let p = dir.path();
        let head = git(p, &["rev-parse", "HEAD"]);
        let content = read_to_string(p.join("a.txt")).unwrap();
        let out = CliMerge::new(p, "feature").run(&ctx()).await.unwrap();
        assert_eq!(out.conflicted_files, vec!["a.txt".to_string()]);

        let out = CliMergeAbort::new(p).run(&ctx()).await.unwrap();
        assert!(out.aborted);
        assert_eq!(git(p, &["rev-parse", "HEAD"]), head);
        assert_eq!(git(p, &["status", "--porcelain"]), "");
        assert_eq!(read_to_string(p.join("a.txt")).unwrap(), content);
        assert!(!merge_head(p));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn merge_abort_without_merge_in_progress_is_an_error() {
    timeout(LIMIT, async {
        let dir = repo(false);
        let err = CliMergeAbort::new(dir.path())
            .run(&ctx())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no merge in progress"));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn conflicting_merge_in_blobless_clone_is_concluded() {
    timeout(LIMIT, async {
        let server = GitHttpServer::start_default().await;
        server.seed("repo.git");
        let bare = server.root().join("repo.git");
        let dir = tempdir().unwrap();

        // Conflicting commits on `main` and `feature`, pushed to the server.
        let seed = dir.path().join("seed");
        create_dir(&seed).unwrap();
        let bare_str = bare.to_str().unwrap();
        git(&seed, &["clone", "-q", bare_str, "."]);
        write(seed.join("a.txt"), "base\nline 2\n").unwrap();
        git(&seed, &["add", "."]);
        git(&seed, &["commit", "-q", "-m", "base"]);
        git(&seed, &["checkout", "-q", "-b", "feature"]);
        write(seed.join("a.txt"), "feature\nline 2\n").unwrap();
        git(&seed, &["commit", "-q", "-am", "feature"]);
        git(&seed, &["checkout", "-q", "main"]);
        write(seed.join("a.txt"), "main\nline 2\n").unwrap();
        git(&seed, &["commit", "-q", "-am", "main"]);
        git(&seed, &["push", "-q", bare_str, "main", "feature"]);
        server.allow_filter("repo.git");

        let clone = dir.path().join("clone");
        let op = CliClone::new(server.url("repo.git"), &clone)
            .blobless(true)
            .checkout(false);
        op.run(&ctx_with_token()).await.unwrap();
        let wt = dir.path().join("wt");
        CliWorktreeAdd::new(&clone, &wt, "main")
            .run(&ctx_with_token())
            .await
            .unwrap();

        let out = CliMerge::new(&wt, "origin/feature")
            .run(&ctx_with_token())
            .await
            .unwrap();
        assert_eq!(out.conflicted_files, vec!["a.txt".to_string()]);

        // The first conflicting merge is undone...
        CliMergeAbort::new(&wt)
            .run(&ctx_with_token())
            .await
            .unwrap();
        assert_eq!(git(&wt, &["status", "--porcelain"]), "");

        // ...the second one is concluded.
        let out = CliMerge::new(&wt, "origin/feature")
            .run(&ctx_with_token())
            .await
            .unwrap();
        assert!(out.has_conflicts);
        resolve(&wt).await;
        let op = CliMergeCommit::new(&wt, "Bot", "bot@example.com");
        let commit = op.run(&ctx_with_token()).await.unwrap();
        let parents = git(&wt, &["rev-list", "--parents", "-n", "1", &commit.commit]);
        assert_eq!(parents.split(' ').count(), 3);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn merge_error_never_contains_the_token() {
    timeout(LIMIT, async {
        let server = GitHttpServer::start_default().await;
        server.seed("repo.git");
        let bare = server.root().join("repo.git");
        let bare_str = bare.to_str().unwrap();
        let dir = tempdir().unwrap();
        let seed = dir.path().join("seed");
        create_dir(&seed).unwrap();
        git(&seed, &["clone", "-q", bare_str, "."]);
        write(seed.join("a.txt"), "base\nline 2\n").unwrap();
        git(&seed, &["add", "."]);
        git(&seed, &["commit", "-q", "-m", "base"]);
        git(&seed, &["checkout", "-q", "-b", "feature"]);
        write(seed.join("a.txt"), "feature\nline 2\n").unwrap();
        git(&seed, &["commit", "-q", "-am", "feature"]);
        git(&seed, &["checkout", "-q", "main"]);
        git(&seed, &["push", "-q", bare_str, "main", "feature"]);
        server.allow_filter("repo.git");

        let clone = dir.path().join("clone");
        let op = CliClone::new(server.url("repo.git"), &clone)
            .blobless(true)
            .checkout(false);
        op.run(&ctx_with_token()).await.unwrap();
        let wt = dir.path().join("wt");
        CliWorktreeAdd::new(&clone, &wt, "main")
            .run(&ctx_with_token())
            .await
            .unwrap();

        // The commits are known but the feature blobs are not, and the
        // remote is now unreachable: the authenticated lazy fetch fails.
        let dead = ["remote", "set-url", "origin", "http://127.0.0.1:1/repo.git"];
        git(&clone, &dead);
        let err = CliMerge::new(&wt, "origin/feature")
            .run(&ctx_with_token())
            .await
            .unwrap_err();
        assert!(!err.to_string().contains(TOKEN));
    })
    .await
    .expect("test timed out");
}
