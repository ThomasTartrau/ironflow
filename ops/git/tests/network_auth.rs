//! Network operations authenticate against an HTTPS-style remote with a token
//! read from the workflow secret store.

mod common;

use std::time::Duration;

use common::{
    GitHttpServer, TOKEN, checkout_with_submodule, ctx_with_secret, ctx_with_token,
    ctx_without_secret, local_with_origin,
};
use git2::Repository;
use ironflow_ops_git::fetch::{FetchRemote, PushRemote, RemoteDefaultBranch, RemotePrune};
use ironflow_ops_git::repository::RepoClone;
use ironflow_ops_git::submodule::SubmoduleUpdate;
use tokio::time::timeout;

const LIMIT: Duration = Duration::from_secs(30);

#[tokio::test]
async fn harness_accepts_credentials_embedded_in_url() {
    timeout(LIMIT, async {
        let server = GitHttpServer::start_default().await;
        server.seed("repo.git");
        let target = tempfile::tempdir().unwrap();
        let path = target.path().join("clone");

        RepoClone::new(server.url_with_userinfo("repo.git", "oauth2", TOKEN), &path)
            .run(&ctx_without_secret())
            .await
            .unwrap();

        assert!(path.join("file.txt").exists());
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn repo_clone_of_private_http_repo_authenticates_with_git_token_secret() {
    timeout(LIMIT, async {
        let server = GitHttpServer::start_default().await;
        let head = server.seed("repo.git");
        let target = tempfile::tempdir().unwrap();
        let path = target.path().join("clone");

        let result = RepoClone::new(server.url("repo.git"), &path)
            .run(&ctx_with_token())
            .await
            .unwrap();

        assert_eq!(result.path, path);
        let cloned = Repository::open(&path).unwrap();
        assert_eq!(cloned.head().unwrap().target(), Some(head));
        assert!(path.join("file.txt").exists());
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn repo_clone_uses_custom_secret_key_and_username() {
    timeout(LIMIT, async {
        let server = GitHttpServer::start("ci-bot", "custom-token-value").await;
        server.seed("repo.git");
        let target = tempfile::tempdir().unwrap();
        let path = target.path().join("clone");

        RepoClone::new(server.url("repo.git"), &path)
            .token_secret("my_git_token")
            .username("ci-bot")
            .run(&ctx_with_secret("my_git_token", "custom-token-value"))
            .await
            .unwrap();

        assert!(path.join("file.txt").exists());
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn empty_git_token_secret_is_treated_as_absent() {
    let target = tempfile::tempdir().unwrap();
    let missing = target.path().join("no-such-origin");

    let err = RepoClone::new(missing.to_str().unwrap(), target.path().join("clone"))
        .run(&ctx_with_secret("git_token", ""))
        .await
        .unwrap_err()
        .to_string();

    // Scrubbing an empty token would splice `***` between every character.
    assert!(!err.contains("***"), "got: {err}");
}

#[tokio::test]
async fn rejected_token_fails_fast_naming_the_secret_not_its_value() {
    timeout(LIMIT, async {
        let server = GitHttpServer::start_default().await;
        server.seed("repo.git");
        let target = tempfile::tempdir().unwrap();
        let wrong = "wrong-token-value-42";

        let err = RepoClone::new(server.url("repo.git"), target.path().join("clone"))
            .run(&ctx_with_secret("git_token", wrong))
            .await
            .unwrap_err()
            .to_string();

        assert!(err.contains("`git_token`"), "got: {err}");
        assert!(!err.contains(wrong), "token leaked: {err}");
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn fetch_remote_authenticates_with_git_token_secret() {
    timeout(LIMIT, async {
        let server = GitHttpServer::start_default().await;
        let head = server.seed("repo.git");
        let work = local_with_origin(&server.url("repo.git"));

        FetchRemote::new(
            work.path(),
            "origin",
            vec!["refs/heads/main:refs/remotes/origin/main"],
        )
        .run(&ctx_with_token())
        .await
        .unwrap();

        let repo = Repository::open(work.path()).unwrap();
        let fetched = repo.find_reference("refs/remotes/origin/main").unwrap();
        assert_eq!(fetched.target(), Some(head));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn push_remote_authenticates_with_git_token_secret() {
    timeout(LIMIT, async {
        let server = GitHttpServer::start_default().await;
        server.seed("repo.git");
        let work = local_with_origin(&server.url("repo.git"));
        let local_head = Repository::open(work.path())
            .unwrap()
            .head()
            .unwrap()
            .target()
            .unwrap();

        PushRemote::new(
            work.path(),
            "origin",
            vec!["refs/heads/main:refs/heads/feature"],
        )
        .run(&ctx_with_token())
        .await
        .unwrap();

        let served = Repository::open_bare(server.root().join("repo.git")).unwrap();
        let pushed = served.find_reference("refs/heads/feature").unwrap();
        assert_eq!(pushed.target(), Some(local_head));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn remote_prune_authenticates_and_removes_stale_tracking_refs() {
    timeout(LIMIT, async {
        let server = GitHttpServer::start_default().await;
        let head = server.seed("repo.git");
        server.create_branch("repo.git", "stale", head);
        let work = local_with_origin(&server.url("repo.git"));
        FetchRemote::new(
            work.path(),
            "origin",
            vec!["+refs/heads/*:refs/remotes/origin/*"],
        )
        .run(&ctx_with_token())
        .await
        .unwrap();
        let repo = Repository::open(work.path()).unwrap();
        assert!(repo.find_reference("refs/remotes/origin/stale").is_ok());
        server.delete_branch("repo.git", "stale");

        let result = RemotePrune::new(work.path(), "origin")
            .run(&ctx_with_token())
            .await
            .unwrap();

        assert!(result.pruned);
        assert!(repo.find_reference("refs/remotes/origin/stale").is_err());
        assert!(repo.find_reference("refs/remotes/origin/main").is_ok());
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn remote_default_branch_authenticates_and_reports_head() {
    timeout(LIMIT, async {
        let server = GitHttpServer::start_default().await;
        server.seed("repo.git");
        let work = local_with_origin(&server.url("repo.git"));

        let result = RemoteDefaultBranch::new(work.path(), "origin")
            .run(&ctx_with_token())
            .await
            .unwrap();

        assert_eq!(result.default_branch.as_deref(), Some("refs/heads/main"));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn submodule_update_clones_private_http_submodule_with_git_token_secret() {
    timeout(LIMIT, async {
        let server = GitHttpServer::start_default().await;
        let sub_head = server.seed("sub.git");
        let (_dirs, clone_path) = checkout_with_submodule(&server, &server.url("sub.git"));

        let result = SubmoduleUpdate::new(&clone_path, "vendor/sub")
            .run(&ctx_with_token())
            .await
            .unwrap();

        assert!(result.updated);
        let updated = Repository::open(clone_path.join("vendor/sub")).unwrap();
        assert_eq!(updated.head().unwrap().target(), Some(sub_head));
        assert!(clone_path.join("vendor/sub/file.txt").exists());
    })
    .await
    .expect("test timed out");
}
