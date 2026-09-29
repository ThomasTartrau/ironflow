//! A token, whether it comes from the secret store or is embedded in a URL,
//! never reaches what a step record persists: `input()`, the serialized
//! output, or the error message.

mod common;

use std::time::Duration;

use common::{
    GitHttpServer, TOKEN, USERNAME, checkout_with_submodule, ctx_with_secret, ctx_with_token,
    ctx_without_secret, local_with_origin,
};
use git2::Repository;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext};
use ironflow_ops_git::fetch::{FetchRemote, PushRemote, RemoteDefaultBranch, RemotePrune};
use ironflow_ops_git::remote::{RemoteCreate, RemoteList, RemoteLookup, RemoteSetUrl};
use ironflow_ops_git::repository::RepoClone;
use ironflow_ops_git::submodule::{SubmoduleList, SubmoduleLookup, SubmoduleUpdate};
use serde_json::Value;
use tokio::time::timeout;

const LIMIT: Duration = Duration::from_secs(30);

/// Run `op` and assert `secret` appears neither in its input, nor in its
/// serialized output, nor in its error (display and debug forms).
async fn run_checked(
    op: &dyn Operation,
    ctx: &OperationContext,
    secret: &str,
) -> Result<Value, OperationError> {
    let input = op.input().map(|v| v.to_string()).unwrap_or_default();
    assert!(!input.contains(secret), "token leaked in input: {input}");
    let result = op.execute(ctx).await;
    match &result {
        Ok(output) => {
            let output = output.to_string();
            assert!(!output.contains(secret), "token leaked in output: {output}");
        }
        Err(err) => {
            let shown = format!("{err} / {err:?}");
            assert!(!shown.contains(secret), "token leaked in error: {shown}");
        }
    }
    result
}

fn assert_masked(value: &Value) {
    assert!(
        value.to_string().contains("***@127.0.0.1"),
        "URL not kept in masked form: {value}"
    );
}

#[tokio::test]
async fn token_never_leaks_from_repo_clone() {
    timeout(LIMIT, async {
        let server = GitHttpServer::start_default().await;
        server.seed("repo.git");
        let dirs = tempfile::tempdir().unwrap();

        let embedded = RepoClone::new(
            server.url_with_userinfo("repo.git", USERNAME, TOKEN),
            dirs.path().join("embedded"),
        );
        let output = run_checked(&embedded, &ctx_without_secret(), TOKEN)
            .await
            .unwrap();
        assert_masked(&output);
        assert_masked(&embedded.input().unwrap());

        let from_secret = RepoClone::new(server.url("repo.git"), dirs.path().join("secret"));
        run_checked(&from_secret, &ctx_with_token(), TOKEN)
            .await
            .unwrap();

        let missing = RepoClone::new(
            server.url_with_userinfo("missing.git", USERNAME, TOKEN),
            dirs.path().join("missing"),
        );
        run_checked(&missing, &ctx_without_secret(), TOKEN)
            .await
            .unwrap_err();
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn token_never_leaks_from_fetch_remote() {
    timeout(LIMIT, async {
        let server = GitHttpServer::start_default().await;
        server.seed("repo.git");
        let refspecs = vec!["refs/heads/main:refs/remotes/origin/main"];

        let embedded = local_with_origin(&server.url_with_userinfo("repo.git", USERNAME, TOKEN));
        let op = FetchRemote::new(embedded.path(), "origin", refspecs.clone());
        run_checked(&op, &ctx_without_secret(), TOKEN)
            .await
            .unwrap();

        let plain = local_with_origin(&server.url("repo.git"));
        let op = FetchRemote::new(plain.path(), "origin", refspecs.clone());
        run_checked(&op, &ctx_with_token(), TOKEN).await.unwrap();

        let missing = local_with_origin(&server.url_with_userinfo("missing.git", USERNAME, TOKEN));
        let op = FetchRemote::new(missing.path(), "origin", refspecs);
        run_checked(&op, &ctx_without_secret(), TOKEN)
            .await
            .unwrap_err();
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn token_never_leaks_from_push_remote() {
    timeout(LIMIT, async {
        let server = GitHttpServer::start_default().await;
        server.seed("repo.git");
        let refspecs = vec!["refs/heads/main:refs/heads/pushed"];

        let embedded = local_with_origin(&server.url_with_userinfo("repo.git", USERNAME, TOKEN));
        let op = PushRemote::new(embedded.path(), "origin", refspecs.clone());
        run_checked(&op, &ctx_without_secret(), TOKEN)
            .await
            .unwrap();

        let plain = local_with_origin(&server.url("repo.git"));
        let op = PushRemote::new(
            plain.path(),
            "origin",
            vec!["refs/heads/main:refs/heads/p2"],
        );
        run_checked(&op, &ctx_with_token(), TOKEN).await.unwrap();

        let wrong = "wrong-push-token-value";
        let op = PushRemote::new(plain.path(), "origin", refspecs);
        run_checked(&op, &ctx_with_secret("git_token", wrong), wrong)
            .await
            .unwrap_err();
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn token_never_leaks_from_remote_prune() {
    timeout(LIMIT, async {
        let server = GitHttpServer::start_default().await;
        server.seed("repo.git");

        let embedded = local_with_origin(&server.url_with_userinfo("repo.git", USERNAME, TOKEN));
        let op = RemotePrune::new(embedded.path(), "origin");
        run_checked(&op, &ctx_without_secret(), TOKEN)
            .await
            .unwrap();

        let plain = local_with_origin(&server.url("repo.git"));
        let op = RemotePrune::new(plain.path(), "origin");
        run_checked(&op, &ctx_with_token(), TOKEN).await.unwrap();

        let missing = local_with_origin(&server.url_with_userinfo("missing.git", USERNAME, TOKEN));
        let op = RemotePrune::new(missing.path(), "origin");
        run_checked(&op, &ctx_without_secret(), TOKEN)
            .await
            .unwrap_err();
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn token_never_leaks_from_remote_default_branch() {
    timeout(LIMIT, async {
        let server = GitHttpServer::start_default().await;
        server.seed("repo.git");

        let embedded = local_with_origin(&server.url_with_userinfo("repo.git", USERNAME, TOKEN));
        let op = RemoteDefaultBranch::new(embedded.path(), "origin");
        run_checked(&op, &ctx_without_secret(), TOKEN)
            .await
            .unwrap();

        let plain = local_with_origin(&server.url("repo.git"));
        let op = RemoteDefaultBranch::new(plain.path(), "origin");
        run_checked(&op, &ctx_with_token(), TOKEN).await.unwrap();

        let wrong = "wrong-default-branch-token";
        run_checked(&op, &ctx_with_secret("git_token", wrong), wrong)
            .await
            .unwrap_err();
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn token_never_leaks_from_submodule_update() {
    timeout(LIMIT, async {
        let server = GitHttpServer::start_default().await;
        server.seed("sub.git");

        let (_ok_dirs, ok_clone) = checkout_with_submodule(&server, &server.url("sub.git"));
        let op = SubmoduleUpdate::new(&ok_clone, "vendor/sub");
        run_checked(&op, &ctx_with_token(), TOKEN).await.unwrap();

        let (_ko_dirs, ko_clone) = checkout_with_submodule(&server, &server.url("sub.git"));
        let wrong = "wrong-submodule-token";
        let op = SubmoduleUpdate::new(&ko_clone, "vendor/sub");
        run_checked(&op, &ctx_with_secret("git_token", wrong), wrong)
            .await
            .unwrap_err();
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn token_never_leaks_from_submodule_lookup_and_list() {
    timeout(LIMIT, async {
        let server = GitHttpServer::start_default().await;
        server.seed("sub.git");
        let url = server.url_with_userinfo("sub.git", USERNAME, TOKEN);
        let (_dirs, clone) = checkout_with_submodule(&server, &url);

        let lookup = SubmoduleLookup::new(&clone, "vendor/sub");
        assert_masked(
            &run_checked(&lookup, &ctx_without_secret(), TOKEN)
                .await
                .unwrap(),
        );
        let list = SubmoduleList::new(&clone);
        assert_masked(
            &run_checked(&list, &ctx_without_secret(), TOKEN)
                .await
                .unwrap(),
        );
    })
    .await
    .expect("test timed out");
}

const TOKEN_URL: &str = "https://oauth2:glpat-test-token-0123456789@gitlab.example.com/g/r.git";

fn repo_with_token_remote() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let repo = Repository::init(tmp.path()).unwrap();
    repo.remote("origin", TOKEN_URL).unwrap();
    repo.remote_set_pushurl("origin", Some(TOKEN_URL)).unwrap();
    tmp
}

fn assert_masked_https(value: &Value) {
    assert!(
        value
            .to_string()
            .contains("https://***@gitlab.example.com/g/r.git"),
        "URL not kept in masked form: {value}"
    );
}

#[tokio::test]
async fn token_never_leaks_from_remote_create() {
    let tmp = tempfile::tempdir().unwrap();
    Repository::init(tmp.path()).unwrap();
    let op = RemoteCreate::new(tmp.path(), "origin", TOKEN_URL);

    assert_masked_https(&op.input().unwrap());
    let output = run_checked(&op, &ctx_without_secret(), TOKEN)
        .await
        .unwrap();
    assert_masked_https(&output);
    // Creating the same remote twice fails.
    run_checked(&op, &ctx_without_secret(), TOKEN)
        .await
        .unwrap_err();
}

#[tokio::test]
async fn token_never_leaks_from_remote_set_url() {
    let tmp = repo_with_token_remote();
    let op = RemoteSetUrl::new(tmp.path(), "origin", TOKEN_URL);

    assert_masked_https(&op.input().unwrap());
    let output = run_checked(&op, &ctx_without_secret(), TOKEN)
        .await
        .unwrap();
    assert_masked_https(&output);

    let not_a_repo = tempfile::tempdir().unwrap();
    let op = RemoteSetUrl::new(not_a_repo.path(), "origin", TOKEN_URL);
    run_checked(&op, &ctx_without_secret(), TOKEN)
        .await
        .unwrap_err();
}

#[tokio::test]
async fn token_never_leaks_from_remote_list() {
    let tmp = repo_with_token_remote();
    let op = RemoteList::new(tmp.path());
    let output = run_checked(&op, &ctx_without_secret(), TOKEN)
        .await
        .unwrap();
    assert_masked_https(&output);

    let not_a_repo = tempfile::tempdir().unwrap();
    run_checked(
        &RemoteList::new(not_a_repo.path()),
        &ctx_without_secret(),
        TOKEN,
    )
    .await
    .unwrap_err();
}

#[tokio::test]
async fn token_never_leaks_from_remote_lookup() {
    let tmp = repo_with_token_remote();
    let op = RemoteLookup::new(tmp.path(), "origin");
    let output = run_checked(&op, &ctx_without_secret(), TOKEN)
        .await
        .unwrap();
    assert_masked_https(&output);
    assert_eq!(
        output["pushurl"], "https://***@gitlab.example.com/g/r.git",
        "pushurl must be masked too"
    );

    let op = RemoteLookup::new(tmp.path(), "missing");
    run_checked(&op, &ctx_without_secret(), TOKEN)
        .await
        .unwrap_err();
}
