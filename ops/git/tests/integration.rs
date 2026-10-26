//! Integration tests for ironflow-ops-git.

use std::fs;
#[cfg(unix)]
use std::fs::OpenOptions;
#[cfg(unix)]
use std::io::Write;
use std::path::Path;
use std::sync::Arc;

use git2::{Repository, Signature, Version};
use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_git::blame::BlameFile;
use ironflow_ops_git::branch::{BranchCheckout, BranchCreate, BranchList};
use ironflow_ops_git::config::ConfigList;
use ironflow_ops_git::diff::{DiffAgainstRef, DiffIndexToWorkdir};
use ironflow_ops_git::index::IndexAdd;
use ironflow_ops_git::reflog::{ReflogAppend, ReflogRead};
use ironflow_ops_git::remote::{RemoteList, RemoteLookup};
use ironflow_ops_git::status::StatusList;
use ironflow_ops_git::submodule::{SubmoduleList, SubmoduleLookup};
use ironflow_ops_git::tag::{TagCreateLightweight, TagList};
use ironflow_ops_git::worktree::{WorktreeAdd, WorktreeList, WorktreeRemove};

fn ctx() -> OperationContext {
    OperationContext::new(Arc::new(NoopSecretResolver))
}

fn init_repo_with_commit(path: &Path) {
    let repo = Repository::init(path).unwrap();
    fs::write(path.join("file.txt"), "content").unwrap();
    let mut index = repo.index().unwrap();
    index.add_path(Path::new("file.txt")).unwrap();
    index.write().unwrap();
    let tree_oid = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_oid).unwrap();
    let sig = Signature::now("Test", "test@test.com").unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
        .unwrap();
}

#[tokio::test]
async fn branch_create_and_list() {
    let tmp = tempfile::tempdir().unwrap();
    init_repo_with_commit(tmp.path());

    let create = BranchCreate::new(tmp.path(), "feature");
    let result = create.run(&ctx()).await.unwrap();
    assert_eq!(result.name, "feature");

    let list = BranchList::local(tmp.path());
    let result = list.run(&ctx()).await.unwrap();
    let names: Vec<&str> = result.branches.iter().map(|b| b.name.as_str()).collect();
    assert!(names.contains(&"feature"));
    assert!(names.iter().any(|n| *n == "main" || *n == "master"));
}

#[tokio::test]
async fn branch_checkout_switches_head() {
    let tmp = tempfile::tempdir().unwrap();
    init_repo_with_commit(tmp.path());

    BranchCreate::new(tmp.path(), "feature")
        .run(&ctx())
        .await
        .unwrap();

    let result = BranchCheckout::new(tmp.path(), "feature")
        .run(&ctx())
        .await
        .unwrap();
    assert_eq!(result.name, "feature");

    let repo = Repository::open(tmp.path()).unwrap();
    let head = repo.head().unwrap();
    assert!(head.is_branch());
    assert_eq!(
        head.shorthand().unwrap(),
        "feature",
        "HEAD should point to the checked-out branch"
    );
}

#[tokio::test]
async fn branch_checkout_nonexistent_fails() {
    let tmp = tempfile::tempdir().unwrap();
    init_repo_with_commit(tmp.path());

    let result = BranchCheckout::new(tmp.path(), "does-not-exist")
        .run(&ctx())
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn tag_create_and_list() {
    let tmp = tempfile::tempdir().unwrap();
    init_repo_with_commit(tmp.path());

    let create = TagCreateLightweight::new(tmp.path(), "v1.0.0");
    let result = create.run(&ctx()).await.unwrap();
    assert_eq!(result.name, "v1.0.0");

    let list = TagList::new(tmp.path());
    let result = list.run(&ctx()).await.unwrap();
    assert!(result.tags.contains(&"v1.0.0".to_string()));
}

#[tokio::test]
async fn status_list_shows_changes() {
    let tmp = tempfile::tempdir().unwrap();
    init_repo_with_commit(tmp.path());

    fs::write(tmp.path().join("new.txt"), "new file").unwrap();
    fs::write(tmp.path().join("file.txt"), "modified").unwrap();

    let status = StatusList::new(tmp.path());
    let result = status.run(&ctx()).await.unwrap();
    let paths: Vec<&str> = result.entries.iter().map(|e| e.path.as_str()).collect();
    assert!(paths.contains(&"new.txt"));
    assert!(paths.contains(&"file.txt"));
}

#[tokio::test]
async fn diff_index_to_workdir() {
    let tmp = tempfile::tempdir().unwrap();
    init_repo_with_commit(tmp.path());

    fs::write(tmp.path().join("file.txt"), "modified content").unwrap();

    let diff = DiffIndexToWorkdir::new(tmp.path());
    let result = diff.run(&ctx()).await.unwrap();
    assert!(result.files_changed > 0);
    assert!(result.insertions > 0 || result.deletions > 0);
}

#[tokio::test]
async fn all_ops_kind_git() {
    use ironflow_ops_git::commit::CommitCreate;
    use ironflow_ops_git::remote::RemoteCreate;
    use ironflow_ops_git::repository::RepoInit;

    let init: &dyn Operation = &RepoInit::new("/tmp/x", false);
    let add: &dyn Operation = &IndexAdd::new("/tmp/x", "f");
    let commit: &dyn Operation = &CommitCreate::new("/tmp/x", "m", "n", "e");
    let branch: &dyn Operation = &BranchCreate::new("/tmp/x", "b");
    let tag: &dyn Operation = &TagCreateLightweight::new("/tmp/x", "t");
    let remote: &dyn Operation = &RemoteCreate::new("/tmp/x", "o", "u");
    let status: &dyn Operation = &StatusList::new("/tmp/x");
    let diff: &dyn Operation = &DiffIndexToWorkdir::new("/tmp/x");
    let against: &dyn Operation = &DiffAgainstRef::new("/tmp/x", "origin/main");

    for op in [
        init, add, commit, branch, tag, remote, status, diff, against,
    ] {
        assert_eq!(op.kind(), "git");
    }
}

#[tokio::test]
async fn ops_provide_input() {
    use ironflow_ops_git::commit::CommitCreate;
    use ironflow_ops_git::repository::RepoInit;

    let init = RepoInit::new("/tmp/test", false);
    let input = init.input().unwrap();
    assert!(input["path"].is_string());
    assert_eq!(input["bare"], false);

    let commit = CommitCreate::new("/tmp/test", "msg", "Name", "email@test.com");
    let input = commit.input().unwrap();
    assert_eq!(input["message"], "msg");
    assert!(input["author"].is_string());

    let against = DiffAgainstRef::new("/tmp/test", "origin/main");
    let input = against.input().unwrap();
    assert_eq!(input["repo_path"], "/tmp/test");
    assert_eq!(input["base_ref"], "origin/main");
}

#[test]
fn libgit2_is_built_with_ssh_and_https() {
    let version = Version::get();
    assert!(version.ssh());
    assert!(version.https());
}

#[cfg(unix)]
fn append_to_git_config(repo_path: &Path, bytes: &[u8]) {
    let mut file = OpenOptions::new()
        .append(true)
        .open(repo_path.join(".git").join("config"))
        .unwrap();
    file.write_all(bytes).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn config_list_skips_entries_with_non_utf8_value() {
    let tmp = tempfile::tempdir().unwrap();
    init_repo_with_commit(tmp.path());
    append_to_git_config(tmp.path(), b"[x]\n\ty = \xFF\n\tz = valid\n");

    let result = ConfigList::new(tmp.path()).run(&ctx()).await.unwrap();
    assert!(
        result
            .entries
            .iter()
            .any(|e| e.name == "x.z" && e.value == "valid")
    );
    assert!(result.entries.iter().all(|e| e.name != "x.y"));
}

#[cfg(unix)]
#[tokio::test]
async fn remote_list_returns_empty_url_when_url_is_not_utf8() {
    let tmp = tempfile::tempdir().unwrap();
    init_repo_with_commit(tmp.path());
    append_to_git_config(tmp.path(), b"[remote \"bad\"]\n\turl = \xFF\n");

    let list = RemoteList::new(tmp.path()).run(&ctx()).await.unwrap();
    let bad = list.remotes.iter().find(|r| r.name == "bad").unwrap();
    assert_eq!(bad.url, "");

    let got = RemoteLookup::new(tmp.path(), "bad")
        .run(&ctx())
        .await
        .unwrap();
    assert_eq!(got.url, "");
}

#[tokio::test]
async fn remote_list_and_get_return_redacted_url_and_pushurl() {
    let tmp = tempfile::tempdir().unwrap();
    init_repo_with_commit(tmp.path());
    let repo = Repository::open(tmp.path()).unwrap();
    repo.remote("origin", "https://user:secret@example.com/repo.git")
        .unwrap();
    repo.remote_set_pushurl("origin", Some("https://user:secret@example.com/push.git"))
        .unwrap();

    let list = RemoteList::new(tmp.path()).run(&ctx()).await.unwrap();
    assert_eq!(list.remotes.len(), 1);
    assert_eq!(list.remotes[0].name, "origin");
    assert!(!list.remotes[0].url.contains("secret"));
    assert!(list.remotes[0].url.contains("example.com/repo.git"));

    let got = RemoteLookup::new(tmp.path(), "origin")
        .run(&ctx())
        .await
        .unwrap();
    assert!(!got.url.contains("secret"));
    let pushurl = got.pushurl.unwrap();
    assert!(!pushurl.contains("secret"));
    assert!(pushurl.contains("example.com/push.git"));
}

#[tokio::test]
async fn submodule_lookup_and_list_return_name_url_and_path() {
    let tmp = tempfile::tempdir().unwrap();
    init_repo_with_commit(tmp.path());
    {
        let repo = Repository::open(tmp.path()).unwrap();
        repo.submodule(
            "https://example.com/org/sub.git",
            Path::new("vendor/sub"),
            false,
        )
        .unwrap();
    }

    let lookup = SubmoduleLookup::new(tmp.path(), "vendor/sub")
        .run(&ctx())
        .await
        .unwrap();
    assert_eq!(lookup.name, "vendor/sub");
    assert_eq!(lookup.url, "https://example.com/org/sub.git");
    assert_eq!(lookup.path, "vendor/sub");

    let list = SubmoduleList::new(tmp.path()).run(&ctx()).await.unwrap();
    assert_eq!(list.submodules.len(), 1);
    assert_eq!(list.submodules[0].name, "vendor/sub");
    assert_eq!(list.submodules[0].url, "https://example.com/org/sub.git");
    assert_eq!(list.submodules[0].path, "vendor/sub");
}

#[tokio::test]
async fn reflog_read_returns_message_and_committer() {
    let tmp = tempfile::tempdir().unwrap();
    init_repo_with_commit(tmp.path());
    let oid = Repository::open(tmp.path())
        .unwrap()
        .head()
        .unwrap()
        .target()
        .unwrap()
        .to_string();
    ReflogAppend::new(tmp.path(), "HEAD", &oid, "custom entry", "Bot", "bot@t.com")
        .run(&ctx())
        .await
        .unwrap();

    let result = ReflogRead::new(tmp.path(), "HEAD", 50)
        .run(&ctx())
        .await
        .unwrap();
    let entry = result
        .entries
        .iter()
        .find(|e| e.message == "custom entry")
        .unwrap();
    assert_eq!(entry.committer, "Bot");
    assert_eq!(entry.id_new, oid);
}

#[tokio::test]
async fn tag_list_returns_exact_names() {
    let tmp = tempfile::tempdir().unwrap();
    init_repo_with_commit(tmp.path());
    TagCreateLightweight::new(tmp.path(), "v1")
        .run(&ctx())
        .await
        .unwrap();
    let list = TagList::new(tmp.path()).run(&ctx()).await.unwrap();
    assert_eq!(list.tags, vec!["v1".to_string()]);
}

#[tokio::test]
async fn worktree_list_and_remove_use_the_worktree_name() {
    let tmp = tempfile::tempdir().unwrap();
    init_repo_with_commit(tmp.path());
    let wt_dir = tempfile::tempdir().unwrap();
    let wt_path = wt_dir.path().join("wt");
    WorktreeAdd::new(tmp.path(), "feature-wt", &wt_path)
        .run(&ctx())
        .await
        .unwrap();

    let list = WorktreeList::new(tmp.path()).run(&ctx()).await.unwrap();
    assert_eq!(list.worktrees, vec!["feature-wt".to_string()]);
    assert_eq!(list.entries.len(), 1);
    let entry = &list.entries[0];
    assert_eq!(entry.name, "feature-wt");
    assert_eq!(entry.branch.as_deref(), Some("feature-wt"));
    assert!(entry.commit.is_some());
    assert!(entry.path.ends_with("wt"));

    let removed = WorktreeRemove::new(tmp.path(), "feature-wt")
        .run(&ctx())
        .await
        .unwrap();
    assert_eq!(removed.name, "feature-wt");
    assert!(removed.removed);
}

#[tokio::test]
async fn config_list_returns_exact_name_and_value() {
    let tmp = tempfile::tempdir().unwrap();
    init_repo_with_commit(tmp.path());
    Repository::open(tmp.path())
        .unwrap()
        .config()
        .unwrap()
        .set_str("ironflow.key", "some value")
        .unwrap();

    let result = ConfigList::new(tmp.path()).run(&ctx()).await.unwrap();
    let entry = result
        .entries
        .iter()
        .find(|e| e.name == "ironflow.key")
        .unwrap();
    assert_eq!(entry.value, "some value");
}

#[tokio::test]
async fn blame_returns_author_name_for_each_hunk() {
    let tmp = tempfile::tempdir().unwrap();
    init_repo_with_commit(tmp.path());

    let result = BlameFile::new(tmp.path(), "file.txt")
        .run(&ctx())
        .await
        .unwrap();
    assert_eq!(result.file, "file.txt");
    assert_eq!(result.hunks.len(), 1);
    assert_eq!(result.hunks[0].author, "Test");
    assert_eq!(result.hunks[0].start_line, 1);
    assert_eq!(result.hunks[0].lines, 1);
}
