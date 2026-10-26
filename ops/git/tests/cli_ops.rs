//! Git CLI operations against a local smart-HTTP server that requires a
//! token: partial clone, fetch, worktree add and origin-checked push.

mod common;

use std::fs::{create_dir, read_dir, read_to_string, write};
use std::path::Path;
use std::process::Command as StdCommand;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use common::{
    GitHttpServer, TOKEN, USERNAME, create_feature, ctx_with_secret, ctx_with_token,
    ctx_without_secret, local_with_origin,
};
use git2::{Config, Oid, Repository, Signature};
use ironflow_ops_git::cli::{CliClone, CliFetch, CliPush, CliWorktreeAdd, PushMode};
use tempfile::tempdir;
use tokio::join;
use tokio::net::TcpListener;
use tokio::spawn;
use tokio::time::{sleep, timeout};

const LIMIT: Duration = Duration::from_secs(30);

/// A remote no test talks to: the operations under test fail before.
const ELSEWHERE: &str = "https://gitlab.example.com/g/r.git";

/// Ref of the branch the push tests push.
const FEATURE: &str = "refs/heads/feature";

/// Whether `parent` holds no staging directory of a clone.
fn no_staging_left(parent: &Path) -> bool {
    for entry in read_dir(parent).unwrap() {
        let name = entry.unwrap().file_name();
        if name.to_string_lossy().contains("ironflow-clone") {
            return false;
        }
    }
    true
}

/// Target of the served ref `name` in the served repository `repo`.
fn served_ref(server: &GitHttpServer, repo: &str, name: &str) -> Option<Oid> {
    let repo = Repository::open_bare(server.root().join(repo)).unwrap();
    repo.refname_to_id(name).ok()
}

/// Run `git` with `args` and return its trimmed stdout.
fn git(args: &[&str]) -> String {
    let output = StdCommand::new("git").args(args).output().unwrap();
    assert!(output.status.success(), "git {args:?} failed");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// Point the local branch `feature` at a new root commit, unrelated to
/// what was pushed before.
fn rewrite_feature(work: &Path, message: &str) -> Oid {
    let repo = Repository::open(work).unwrap();
    let tree = repo.head().unwrap().peel_to_tree().unwrap();
    let sig = Signature::now("Test", "test@test.com").unwrap();
    let oid = repo.commit(None, &sig, &sig, message, &tree, &[]).unwrap();
    let commit = repo.find_commit(oid).unwrap();
    repo.branch("feature", &commit, true).unwrap();
    oid
}

#[tokio::test]
async fn cli_clone_of_private_repo_writes_no_token_to_config_nor_errors() {
    timeout(LIMIT, async {
        let server = GitHttpServer::start_default().await;
        server.seed("repo.git");
        let dir = tempdir().unwrap();
        let target = dir.path().join("clone");

        let op = CliClone::new(server.url("repo.git"), &target);
        let output = op.run(&ctx_with_token()).await.unwrap();

        assert!(target.join("file.txt").exists());
        assert!(!output.reused);
        assert!(!output.url.contains(TOKEN));
        let config = read_to_string(target.join(".git").join("config")).unwrap();
        assert!(!config.contains(TOKEN), "got: {config}");
        assert!(!config.contains("credential"), "got: {config}");
        assert!(no_staging_left(dir.path()));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn cli_clone_with_explicit_token_authenticates() {
    timeout(LIMIT, async {
        let server = GitHttpServer::start_default().await;
        server.seed("repo.git");
        let dir = tempdir().unwrap();
        let target = dir.path().join("clone");

        let op = CliClone::new(server.url("repo.git"), &target).token(TOKEN);
        op.run(&ctx_without_secret()).await.unwrap();

        assert!(target.join("file.txt").exists());
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn cli_clone_without_any_token_fails_without_prompting() {
    timeout(LIMIT, async {
        let server = GitHttpServer::start_default().await;
        server.seed("repo.git");
        let dir = tempdir().unwrap();
        let target = dir.path().join("clone");

        let op = CliClone::new(server.url("repo.git"), &target);
        op.run(&ctx_without_secret()).await.unwrap_err();

        assert!(!target.exists());
        assert!(no_staging_left(dir.path()));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn failed_cli_clone_publishes_no_directory() {
    timeout(LIMIT, async {
        let server = GitHttpServer::start_default().await;
        let dir = tempdir().unwrap();
        let target = dir.path().join("clone");

        let op = CliClone::new(server.url("missing.git"), &target);
        let err = op.run(&ctx_with_token()).await.unwrap_err();

        assert!(!err.to_string().contains(TOKEN));
        assert!(!target.exists());
        assert!(no_staging_left(dir.path()));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn interrupted_cli_clone_publishes_no_directory() {
    timeout(LIMIT, async {
        // A server that accepts connections and never answers: git connects
        // and waits forever, so the clone is in flight when it is cancelled.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let accepted = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&accepted);
        let _hold = spawn(async move {
            let mut streams = Vec::new();
            while let Ok((stream, _)) = listener.accept().await {
                counter.fetch_add(1, Ordering::SeqCst);
                streams.push(stream);
            }
        });

        let dir = tempdir().unwrap();
        let parent = dir.path().join("nested");
        let target = parent.join("clone");
        let op = CliClone::new(format!("http://{addr}/repo.git"), &target);
        let ctx = ctx_with_token();
        let cancelled = timeout(Duration::from_secs(2), op.run(&ctx)).await;

        assert!(cancelled.is_err(), "the clone was not interrupted");
        assert!(accepted.load(Ordering::SeqCst) > 0, "git never connected");

        let deadline = Instant::now() + Duration::from_secs(5);
        while target.exists() || !no_staging_left(&parent) {
            assert!(
                Instant::now() < deadline,
                "the interrupted clone left a directory behind"
            );
            sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn cli_clone_rejected_token_leaves_nothing_and_does_not_leak() {
    timeout(LIMIT, async {
        let server = GitHttpServer::start_default().await;
        server.seed("repo.git");
        let dir = tempdir().unwrap();
        let target = dir.path().join("clone");
        let wrong = "wrong-cli-clone-token";

        let op = CliClone::new(server.url("repo.git"), &target);
        let err = op.run(&ctx_with_secret("git_token", wrong)).await;

        let shown = format!("{:?}", err.unwrap_err());
        assert!(!shown.contains(wrong), "token leaked: {shown}");
        assert!(!target.exists());
        assert!(no_staging_left(dir.path()));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn blobless_cli_clone_without_checkout_disables_symlinks() {
    timeout(LIMIT, async {
        let server = GitHttpServer::start_default().await;
        server.seed("repo.git");
        server.allow_filter("repo.git");
        let dir = tempdir().unwrap();
        let target = dir.path().join("clone");

        let op = CliClone::new(server.url("repo.git"), &target)
            .blobless(true)
            .checkout(false)
            .symlinks(false);
        let output = op.run(&ctx_with_token()).await.unwrap();

        assert!(output.blobless && !output.checkout && !output.symlinks);
        let config = Config::open(&target.join(".git").join("config")).unwrap();
        assert!(config.get_bool("remote.origin.promisor").unwrap());
        let filter = config.get_string("remote.origin.partialclonefilter");
        assert_eq!(filter.unwrap(), "blob:none");
        assert!(!config.get_bool("core.symlinks").unwrap());
        assert!(!target.join("file.txt").exists());
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn cli_worktree_add_on_blobless_clone_fetches_blobs_with_token() {
    timeout(LIMIT, async {
        let server = GitHttpServer::start_default().await;
        let head = server.seed("repo.git");
        server.allow_filter("repo.git");
        let dir = tempdir().unwrap();
        let clone = dir.path().join("clone");
        let op = CliClone::new(server.url("repo.git"), &clone)
            .blobless(true)
            .checkout(false);
        op.run(&ctx_with_token()).await.unwrap();

        let wt = dir.path().join("wt");
        let op = CliWorktreeAdd::new(&clone, &wt, "main");
        let output = op.run(&ctx_with_token()).await.unwrap();

        assert_eq!(read_to_string(wt.join("file.txt")).unwrap(), "content");
        assert_eq!(output.commit, head.to_string());
        assert_eq!(output.branch, None);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn cli_worktree_add_creates_the_branch() {
    timeout(LIMIT, async {
        let server = GitHttpServer::start_default().await;
        let head = server.seed("repo.git");
        let dir = tempdir().unwrap();
        let clone = dir.path().join("clone");
        let op = CliClone::new(server.url("repo.git"), &clone);
        op.run(&ctx_with_token()).await.unwrap();

        let wt = dir.path().join("wt");
        let op = CliWorktreeAdd::new(&clone, &wt, "main").branch("review");
        let output = op.run(&ctx_with_token()).await.unwrap();

        assert_eq!(output.branch.as_deref(), Some("review"));
        let wt_path = wt.to_str().unwrap();
        let branch = git(&["-C", wt_path, "rev-parse", "--abbrev-ref", "HEAD"]);
        assert_eq!(branch, "review");
        assert_eq!(output.commit, head.to_string());

        let op = CliWorktreeAdd::new(&clone, dir.path().join("bad"), "main").branch("a..b");
        op.run(&ctx_with_token()).await.unwrap_err();
        let op = CliWorktreeAdd::new(&clone, dir.path().join("bad"), "--orphan");
        op.run(&ctx_with_token()).await.unwrap_err();
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn cli_fetch_updates_tracking_refs_of_partial_clone() {
    timeout(LIMIT, async {
        let server = GitHttpServer::start_default().await;
        let head = server.seed("repo.git");
        server.allow_filter("repo.git");
        let dir = tempdir().unwrap();
        let clone = dir.path().join("clone");
        let op = CliClone::new(server.url("repo.git"), &clone)
            .blobless(true)
            .checkout(false);
        op.run(&ctx_with_token()).await.unwrap();
        server.create_branch("repo.git", "other", head);

        let refspecs = vec!["+refs/heads/*:refs/remotes/origin/*"];
        let op = CliFetch::new(&clone, "origin", refspecs);
        let output = op.run(&ctx_with_token()).await.unwrap();

        assert_eq!(output.remote, "origin");
        let clone_path = clone.to_str().unwrap();
        let tracking = "refs/remotes/origin/other";
        let other = git(&["-C", clone_path, "rev-parse", tracking]);
        assert_eq!(other, head.to_string());
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn cli_fetch_rejects_invalid_remote_name() {
    let dir = tempdir().unwrap();
    for remote in ["", "--upload-pack=touch x", "ext::sh"] {
        let op = CliFetch::new(dir.path(), remote, vec!["main"]);
        op.run(&ctx_without_secret()).await.unwrap_err();
    }
}

#[tokio::test]
async fn cli_clone_refuses_url_with_embedded_credentials() {
    let server = GitHttpServer::start_default().await;
    server.seed("repo.git");
    let dir = tempdir().unwrap();
    let target = dir.path().join("clone");

    let url = server.url_with_userinfo("repo.git", USERNAME, TOKEN);
    let op = CliClone::new(url, &target);
    let err = op.run(&ctx_without_secret()).await;

    let message = err.unwrap_err().to_string();
    assert!(!message.contains(TOKEN), "token leaked: {message}");
    assert!(message.contains("token_secret"), "got: {message}");
    assert!(!target.exists());
}

#[tokio::test]
async fn cli_clone_refuses_option_like_and_transport_helper_urls() {
    let dir = tempdir().unwrap();
    let target = dir.path().join("clone");
    for url in ["--upload-pack=x", "ext::sh -c touch", "http://h/r\nx"] {
        let op = CliClone::new(url, &target);
        op.run(&ctx_without_secret()).await.unwrap_err();
    }
    assert!(!target.exists());
}

#[tokio::test]
async fn cli_clone_into_path_without_file_name_fails() {
    let ctx = ctx_without_secret();
    for target in ["/", ".."] {
        let op = CliClone::new(ELSEWHERE, target);
        let message = op.run(&ctx).await.unwrap_err().to_string();
        assert!(message.contains("has no file name"), "got: {message}");
    }
}

#[tokio::test]
async fn cli_clone_into_non_empty_directory_fails() {
    timeout(LIMIT, async {
        let server = GitHttpServer::start_default().await;
        server.seed("repo.git");
        let dir = tempdir().unwrap();
        let target = dir.path().join("clone");
        create_dir(&target).unwrap();
        write(target.join("keep.txt"), "keep").unwrap();

        let op = CliClone::new(server.url("repo.git"), &target);
        let message = op.run(&ctx_with_token()).await.unwrap_err().to_string();

        assert!(message.contains("already exists"), "got: {message}");
        assert_eq!(read_to_string(target.join("keep.txt")).unwrap(), "keep");
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn cli_clone_into_empty_directory_and_missing_parents_succeeds() {
    timeout(LIMIT, async {
        let server = GitHttpServer::start_default().await;
        server.seed("repo.git");
        let dir = tempdir().unwrap();

        let empty = dir.path().join("empty");
        create_dir(&empty).unwrap();
        let op = CliClone::new(server.url("repo.git"), &empty);
        op.run(&ctx_with_token()).await.unwrap();
        assert!(empty.join("file.txt").exists());

        let nested = dir.path().join("a").join("b").join("clone");
        let op = CliClone::new(server.url("repo.git"), &nested).bare(true);
        let output = op.run(&ctx_with_token()).await.unwrap();
        assert!(output.bare);
        assert!(nested.join("HEAD").exists());
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn cli_clone_reuse_existing_returns_reused_when_origin_matches() {
    timeout(LIMIT, async {
        let server = GitHttpServer::start_default().await;
        server.seed("repo.git");
        let dir = tempdir().unwrap();
        let target = dir.path().join("clone");
        let op = CliClone::new(server.url("repo.git"), &target);
        op.run(&ctx_with_token()).await.unwrap();

        // Same remote spelled differently, and no token: no network call.
        let same = format!("{}/", server.url("repo.git"));
        let op = CliClone::new(same, &target).reuse_existing(true);
        let output = op.run(&ctx_without_secret()).await.unwrap();
        assert!(output.reused);
        assert!(output.checkout);

        let op = CliClone::new(server.url("other.git"), &target).reuse_existing(true);
        let message = op.run(&ctx_with_token()).await.unwrap_err().to_string();
        assert!(message.contains("refusing to reuse"), "got: {message}");
        assert!(target.join("file.txt").exists());

        let not_a_repo = dir.path().join("plain");
        create_dir(&not_a_repo).unwrap();
        write(not_a_repo.join("x"), "x").unwrap();
        let op = CliClone::new(server.url("repo.git"), &not_a_repo).reuse_existing(true);
        op.run(&ctx_with_token()).await.unwrap_err();
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn cli_clone_reuse_existing_refuses_clone_with_other_settings() {
    timeout(LIMIT, async {
        let server = GitHttpServer::start_default().await;
        server.seed("repo.git");
        server.allow_filter("repo.git");
        let dir = tempdir().unwrap();
        let target = dir.path().join("clone");
        let op = CliClone::new(server.url("repo.git"), &target);
        op.run(&ctx_with_token()).await.unwrap();

        // A full clone with symlinks enabled is not what was asked for.
        let op = CliClone::new(server.url("repo.git"), &target)
            .symlinks(false)
            .reuse_existing(true);
        let message = op.run(&ctx_with_token()).await.unwrap_err().to_string();
        assert!(message.contains("refusing to reuse"), "got: {message}");
        assert!(message.contains("symlinks"), "got: {message}");

        let op = CliClone::new(server.url("repo.git"), &target)
            .blobless(true)
            .reuse_existing(true);
        let message = op.run(&ctx_with_token()).await.unwrap_err().to_string();
        assert!(message.contains("blobless"), "got: {message}");

        // The output reports the existing clone, not the request.
        let partial = dir.path().join("partial");
        let op = CliClone::new(server.url("repo.git"), &partial)
            .blobless(true)
            .checkout(false)
            .symlinks(false);
        op.run(&ctx_with_token()).await.unwrap();
        let op = CliClone::new(server.url("repo.git"), &partial)
            .blobless(true)
            .symlinks(false)
            .reuse_existing(true);
        let output = op.run(&ctx_without_secret()).await.unwrap();
        assert!(output.reused);
        assert!(output.blobless);
        assert!(!output.symlinks);
        assert!(!output.checkout);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn parallel_cli_clone_with_reuse_existing_on_same_target_succeeds_once() {
    timeout(LIMIT, async {
        let server = GitHttpServer::start_default().await;
        server.seed("repo.git");
        let dir = tempdir().unwrap();
        let target = dir.path().join("clone");
        let ctx = ctx_with_token();

        let url = server.url("repo.git");
        let a = CliClone::new(&url, &target).reuse_existing(true);
        let b = CliClone::new(&url, &target).reuse_existing(true);
        let (a, b) = join!(a.run(&ctx), b.run(&ctx));

        let (a, b) = (a.unwrap(), b.unwrap());
        assert_ne!(a.reused, b.reused, "one clone must reuse the other");
        assert!(target.join("file.txt").exists());
        assert!(no_staging_left(dir.path()));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn cli_push_is_refused_when_origin_does_not_match_expected_url() {
    timeout(LIMIT, async {
        let server = GitHttpServer::start_default().await;
        server.seed("repo.git");
        let work = local_with_origin(&server.url("repo.git"));
        create_feature(work.path());

        let op = CliPush::new(work.path(), server.url("other.git"), "feature");
        let message = op.run(&ctx_with_token()).await.unwrap_err().to_string();

        assert!(message.contains("refusing to push"), "got: {message}");
        assert_eq!(served_ref(&server, "repo.git", FEATURE), None);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn cli_push_is_refused_when_one_of_several_push_urls_does_not_match() {
    timeout(LIMIT, async {
        let server = GitHttpServer::start_default().await;
        server.seed("repo.git");
        server.seed("other.git");
        let work = local_with_origin(&server.url("repo.git"));
        create_feature(work.path());
        let work_path = work.path().to_str().unwrap();
        let repo_url = server.url("repo.git");
        let other_url = server.url("other.git");
        let key = "remote.origin.pushurl";
        for url in [repo_url.as_str(), other_url.as_str()] {
            git(&["-C", work_path, "config", "--add", key, url]);
        }

        let op = CliPush::new(work.path(), &repo_url, "feature");
        let message = op.run(&ctx_with_token()).await.unwrap_err().to_string();

        assert!(message.contains("refusing to push"), "got: {message}");
        assert!(message.contains(&other_url), "got: {message}");
        assert_eq!(served_ref(&server, "repo.git", FEATURE), None);
        assert_eq!(served_ref(&server, "other.git", FEATURE), None);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn cli_push_is_refused_for_missing_remote_or_local_branch() {
    let work = local_with_origin(ELSEWHERE);
    create_feature(work.path());
    let ctx = ctx_without_secret();

    let op = CliPush::new(work.path(), ELSEWHERE, "feature").remote("missing");
    op.run(&ctx).await.unwrap_err();

    let op = CliPush::new(work.path(), ELSEWHERE, "nope");
    let message = op.run(&ctx).await.unwrap_err().to_string();
    assert!(message.contains("not found"), "got: {message}");
}

#[tokio::test]
async fn cli_push_simple_pushes_branch_to_expected_url() {
    timeout(LIMIT, async {
        let server = GitHttpServer::start_default().await;
        server.seed("repo.git");
        let work = local_with_origin(&server.url("repo.git"));
        let local = create_feature(work.path());

        let expected = format!("{}/", server.url("repo.git"));
        let op = CliPush::new(work.path(), expected, "feature");
        let output = op.run(&ctx_with_token()).await.unwrap();

        let served = served_ref(&server, "repo.git", FEATURE);
        assert_eq!(served, Some(local));
        assert_eq!(output.commit, local.to_string());
        assert_eq!(output.mode, PushMode::Simple);
        assert_eq!(output.remote, "origin");
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn cli_push_force_and_force_with_lease() {
    timeout(LIMIT, async {
        let server = GitHttpServer::start_default().await;
        server.seed("repo.git");
        let url = server.url("repo.git");
        let work = local_with_origin(&url);
        let first = create_feature(work.path());
        let ctx = ctx_with_token();
        let push = |mode: PushMode| CliPush::new(work.path(), &url, "feature").mode(mode);

        push(PushMode::Simple).run(&ctx).await.unwrap();
        let second = rewrite_feature(work.path(), "rewrite 1");

        push(PushMode::Simple).run(&ctx).await.unwrap_err();
        let stale = PushMode::ForceWithLease {
            expected: Some(second.to_string()),
        };
        push(stale).run(&ctx).await.unwrap_err();
        let served = served_ref(&server, "repo.git", FEATURE);
        assert_eq!(served, Some(first));

        let lease = PushMode::ForceWithLease {
            expected: Some(first.to_string()),
        };
        push(lease).run(&ctx).await.unwrap();
        let served = served_ref(&server, "repo.git", FEATURE);
        assert_eq!(served, Some(second));

        let third = rewrite_feature(work.path(), "rewrite 2");
        push(PushMode::Force).run(&ctx).await.unwrap();
        let served = served_ref(&server, "repo.git", FEATURE);
        assert_eq!(served, Some(third));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn cli_push_refuses_expected_url_with_credentials() {
    timeout(LIMIT, async {
        let server = GitHttpServer::start_default().await;
        server.seed("repo.git");
        let work = local_with_origin(&server.url("repo.git"));
        create_feature(work.path());

        let url = server.url_with_userinfo("repo.git", USERNAME, TOKEN);
        let op = CliPush::new(work.path(), url, "feature");
        let message = op.run(&ctx_with_token()).await.unwrap_err().to_string();

        assert!(!message.contains(TOKEN), "token leaked: {message}");
        assert_eq!(served_ref(&server, "repo.git", FEATURE), None);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn cli_push_rejects_invalid_branch_name() {
    let work = local_with_origin(ELSEWHERE);
    let ctx = ctx_without_secret();
    for branch in ["-bad", "a..b", ""] {
        let op = CliPush::new(work.path(), ELSEWHERE, branch);
        let message = op.run(&ctx).await.unwrap_err().to_string();
        assert!(message.contains("invalid branch name"), "got: {message}");
    }
    let lease = PushMode::ForceWithLease {
        expected: Some("not-a-sha".to_string()),
    };
    let op = CliPush::new(work.path(), ELSEWHERE, "main").mode(lease);
    op.run(&ctx).await.unwrap_err();
}

#[tokio::test]
async fn cli_worktree_add_rejects_invalid_branch_name() {
    let work = local_with_origin(ELSEWHERE);
    let ctx = ctx_without_secret();
    let dir = tempdir().unwrap();
    for branch in ["-bad", "a..b", "x y", ""] {
        let op = CliWorktreeAdd::new(work.path(), dir.path().join("wt"), "main").branch(branch);
        let message = op.run(&ctx).await.unwrap_err().to_string();
        assert!(message.contains("invalid branch name"), "got: {message}");
    }
}
