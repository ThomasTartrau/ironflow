//! Commit operations.

use std::path::PathBuf;
use std::str::from_utf8;

use async_trait::async_trait;
use git2::{Commit, Error, Oid, Repository, Signature};
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::helpers::{blocking, prepare_commit, to_value};

/// Output of [`CommitCreate`] and [`CommitAmend`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitOutput {
    /// The OID of the created/amended commit.
    pub oid: String,
    /// The commit message.
    pub message: String,
}

/// Output of [`CommitSigned`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitSignedOutput {
    /// The OID of the signed commit.
    pub oid: String,
    /// The commit message.
    pub message: String,
    /// Whether the commit is signed.
    pub signed: bool,
}

/// Author information returned by [`CommitFind`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitAuthor {
    /// Author name.
    pub name: String,
    /// Author email.
    pub email: String,
}

/// Output of [`CommitFind`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitFindOutput {
    /// The commit OID.
    pub oid: String,
    /// The commit message.
    pub message: String,
    /// Author information.
    pub author: CommitAuthor,
    /// Unix timestamp of the commit.
    pub time: i64,
}

/// Create a new commit on HEAD.
///
/// Stages the current index as a tree, creates a commit with the given
/// message and author, and updates HEAD to point to the new commit.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::commit::CommitCreate;
/// use ironflow_core::operation::Operation;
///
/// let op = CommitCreate::new("/path/to/repo", "Initial commit", "Alice", "alice@example.com");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct CommitCreate {
    repo_path: PathBuf,
    message: String,
    author_name: String,
    author_email: String,
}

impl CommitCreate {
    /// Create a new commit operation.
    pub fn new(
        repo_path: impl Into<PathBuf>,
        message: impl Into<String>,
        author_name: impl Into<String>,
        author_email: impl Into<String>,
    ) -> Self {
        Self {
            repo_path: repo_path.into(),
            message: message.into(),
            author_name: author_name.into(),
            author_email: author_email.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<CommitOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let message = self.message.clone();
        let name = self.author_name.clone();
        let email = self.author_email.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let sig = Signature::now(&name, &email)?;
            let (tree, parents) = prepare_commit(&repo)?;
            let parent_refs: Vec<&Commit<'_>> = parents.iter().collect();

            let oid = repo.commit(Some("HEAD"), &sig, &sig, &message, &tree, &parent_refs)?;
            Ok(CommitOutput {
                oid: oid.to_string(),
                message,
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for CommitCreate {
    fn kind(&self) -> &str {
        "git"
    }

    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "repo_path": self.repo_path,
            "message": self.message,
            "author": format!("{} <{}>", self.author_name, self.author_email),
        }))
    }
}

impl TypedOperation for CommitCreate {
    type Output = CommitOutput;
}

/// Find a commit by its OID.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::commit::CommitFind;
/// use ironflow_core::operation::Operation;
///
/// let op = CommitFind::new("/path/to/repo", "abc123");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct CommitFind {
    repo_path: PathBuf,
    oid: String,
}

impl CommitFind {
    /// Create a new find-commit operation.
    pub fn new(repo_path: impl Into<PathBuf>, oid: impl Into<String>) -> Self {
        Self {
            repo_path: repo_path.into(),
            oid: oid.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<CommitFindOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let oid_str = self.oid.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let oid = Oid::from_str(&oid_str)?;
            let commit = repo.find_commit(oid)?;
            Ok(CommitFindOutput {
                oid: commit.id().to_string(),
                message: commit.message().unwrap_or("").to_string(),
                author: CommitAuthor {
                    name: commit.author().name().unwrap_or("").to_string(),
                    email: commit.author().email().unwrap_or("").to_string(),
                },
                time: commit.time().seconds(),
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for CommitFind {
    fn kind(&self) -> &str {
        "git"
    }

    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "repo_path": self.repo_path, "oid": self.oid }))
    }
}

impl TypedOperation for CommitFind {
    type Output = CommitFindOutput;
}

/// Amend the most recent commit.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::commit::CommitAmend;
/// use ironflow_core::operation::Operation;
///
/// let op = CommitAmend::new("/path/to/repo", "Updated message", "Alice", "alice@example.com");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct CommitAmend {
    repo_path: PathBuf,
    message: String,
    author_name: String,
    author_email: String,
}

impl CommitAmend {
    /// Create a new amend operation.
    pub fn new(
        repo_path: impl Into<PathBuf>,
        message: impl Into<String>,
        author_name: impl Into<String>,
        author_email: impl Into<String>,
    ) -> Self {
        Self {
            repo_path: repo_path.into(),
            message: message.into(),
            author_name: author_name.into(),
            author_email: author_email.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<CommitOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let message = self.message.clone();
        let name = self.author_name.clone();
        let email = self.author_email.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let head = repo.head()?.peel_to_commit()?;
            let sig = Signature::now(&name, &email)?;
            let mut index = repo.index()?;
            let tree_oid = index.write_tree()?;
            let tree = repo.find_tree(tree_oid)?;

            let oid = head.amend(
                Some("HEAD"),
                Some(&sig),
                Some(&sig),
                None,
                Some(&message),
                Some(&tree),
            )?;
            Ok(CommitOutput {
                oid: oid.to_string(),
                message,
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for CommitAmend {
    fn kind(&self) -> &str {
        "git"
    }

    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "repo_path": self.repo_path,
            "message": self.message,
        }))
    }
}

impl TypedOperation for CommitAmend {
    type Output = CommitOutput;
}

/// Create a signed commit (GPG/SSH).
///
/// The signature must be provided as a string. The caller is responsible
/// for generating the signature externally.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::commit::CommitSigned;
/// use ironflow_core::operation::Operation;
///
/// let op = CommitSigned::new(
///     "/path/to/repo",
///     "Signed commit",
///     "Alice",
///     "alice@example.com",
///     "-----BEGIN PGP SIGNATURE-----\n...",
/// );
/// assert_eq!(op.kind(), "git");
/// ```
pub struct CommitSigned {
    repo_path: PathBuf,
    message: String,
    author_name: String,
    author_email: String,
    signature: String,
}

impl CommitSigned {
    /// Create a new signed-commit operation.
    pub fn new(
        repo_path: impl Into<PathBuf>,
        message: impl Into<String>,
        author_name: impl Into<String>,
        author_email: impl Into<String>,
        signature: impl Into<String>,
    ) -> Self {
        Self {
            repo_path: repo_path.into(),
            message: message.into(),
            author_name: author_name.into(),
            author_email: author_email.into(),
            signature: signature.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<CommitSignedOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let message = self.message.clone();
        let name = self.author_name.clone();
        let email = self.author_email.clone();
        let signature = self.signature.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let sig = Signature::now(&name, &email)?;
            let (tree, parents) = prepare_commit(&repo)?;
            let parent_refs: Vec<&Commit<'_>> = parents.iter().collect();

            let buf = repo.commit_create_buffer(&sig, &sig, &message, &tree, &parent_refs)?;
            let content = from_utf8(&buf)
                .map_err(|e| Error::from_str(&format!("invalid UTF-8 in commit buffer: {e}")))?;
            let oid = repo.commit_signed(content, &signature, None)?;

            let head_ref = repo.head()?;
            let mut resolved = head_ref.resolve()?;
            resolved.set_target(oid, "commit signed")?;

            Ok(CommitSignedOutput {
                oid: oid.to_string(),
                message,
                signed: true,
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for CommitSigned {
    fn kind(&self) -> &str {
        "git"
    }

    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "repo_path": self.repo_path,
            "message": self.message,
            "signed": true,
        }))
    }
}

impl TypedOperation for CommitSigned {
    type Output = CommitSignedOutput;
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::*;
    use crate::test_helpers::ctx;

    fn init_repo_with_file(tmp: &Path) -> Repository {
        let repo = Repository::init(tmp).unwrap();
        fs::write(tmp.join("file.txt"), "hello").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("file.txt")).unwrap();
        index.write().unwrap();
        repo
    }

    #[tokio::test]
    async fn add_and_commit() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo_with_file(tmp.path());

        let op = CommitCreate::new(tmp.path(), "test commit", "Test", "test@example.com");
        let result = op.run(&ctx()).await.unwrap();
        assert!(!result.oid.is_empty());
        assert_eq!(result.message, "test commit");

        let repo = Repository::open(tmp.path()).unwrap();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(head.message().unwrap(), "test commit");
    }

    #[tokio::test]
    async fn find_commit_after_create() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo_with_file(tmp.path());

        let create = CommitCreate::new(tmp.path(), "find me", "Test", "test@example.com");
        let result = create.run(&ctx()).await.unwrap();

        let find = CommitFind::new(tmp.path(), &result.oid);
        let found = find.run(&ctx()).await.unwrap();
        assert_eq!(found.message, "find me");
        assert_eq!(found.author.name, "Test");
    }

    #[tokio::test]
    async fn amend_updates_message() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo_with_file(tmp.path());

        let create = CommitCreate::new(tmp.path(), "original", "Test", "test@example.com");
        create.run(&ctx()).await.unwrap();

        let amend = CommitAmend::new(tmp.path(), "amended", "Test", "test@example.com");
        let result = amend.run(&ctx()).await.unwrap();
        assert_eq!(result.message, "amended");

        let repo = Repository::open(tmp.path()).unwrap();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(head.message().unwrap(), "amended");
    }
}
