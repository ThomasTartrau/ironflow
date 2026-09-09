//! Integration tests for ironflow-ops-git.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use git2::{Repository, Signature};
use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_git::branch::{BranchCreate, BranchList};
use ironflow_ops_git::diff::DiffIndexToWorkdir;
use ironflow_ops_git::index::IndexAdd;
use ironflow_ops_git::status::StatusList;
use ironflow_ops_git::tag::{TagCreateLightweight, TagList};

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

    for op in [init, add, commit, branch, tag, remote, status, diff] {
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
}
