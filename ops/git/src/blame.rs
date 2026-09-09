//! Blame operations.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use git2::Repository;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::helpers::{blocking, to_value};

/// A single blame hunk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlameHunk {
    pub commit_id: String,
    pub start_line: usize,
    pub lines: usize,
    pub author: String,
}

/// Output of [`BlameFile`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlameOutput {
    pub file: String,
    pub hunks: Vec<BlameHunk>,
}

/// Blame a file, returning per-line authorship information.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::blame::BlameFile;
/// use ironflow_core::operation::Operation;
///
/// let op = BlameFile::new("/path/to/repo", "src/main.rs");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct BlameFile {
    repo_path: PathBuf,
    file_path: String,
}

impl BlameFile {
    /// Create a new blame operation.
    pub fn new(repo_path: impl Into<PathBuf>, file_path: impl Into<String>) -> Self {
        Self {
            repo_path: repo_path.into(),
            file_path: file_path.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<BlameOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let file_path = self.file_path.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let blame = repo.blame_file(Path::new(&file_path), None)?;
            let hunks: Vec<BlameHunk> = (0..blame.len())
                .filter_map(|i| blame.get_index(i))
                .map(|hunk| BlameHunk {
                    commit_id: hunk.final_commit_id().to_string(),
                    start_line: hunk.final_start_line(),
                    lines: hunk.lines_in_hunk(),
                    author: hunk.final_signature().name().unwrap_or("").to_string(),
                })
                .collect();
            Ok(BlameOutput {
                file: file_path,
                hunks,
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for BlameFile {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "repo_path": self.repo_path, "file": self.file_path }))
    }
}

impl TypedOperation for BlameFile {
    type Output = BlameOutput;
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use git2::{Repository, Signature};
    use ironflow_core::operation::Operation;

    use super::*;
    use crate::test_helpers::ctx;

    fn init_blame_repo(path: &Path) {
        let repo = Repository::init(path).unwrap();
        fs::write(path.join("file.txt"), "line1\nline2\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("file.txt")).unwrap();
        index.write().unwrap();
        let oid = index.write_tree().unwrap();
        let tree = repo.find_tree(oid).unwrap();
        let sig = Signature::now("Alice", "alice@test.com").unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();
    }

    #[tokio::test]
    async fn blame_returns_hunks() {
        let tmp = tempfile::tempdir().unwrap();
        init_blame_repo(tmp.path());
        let result = BlameFile::new(tmp.path(), "file.txt")
            .run(&ctx())
            .await
            .unwrap();
        assert_eq!(result.file, "file.txt");
        assert!(!result.hunks.is_empty());
        assert_eq!(result.hunks[0].author, "Alice");
        assert_eq!(result.hunks[0].start_line, 1);
    }

    #[tokio::test]
    async fn blame_nonexistent_file_fails() {
        let tmp = tempfile::tempdir().unwrap();
        init_blame_repo(tmp.path());
        assert!(
            BlameFile::new(tmp.path(), "nope.txt")
                .run(&ctx())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn execute_serializes_correctly() {
        let tmp = tempfile::tempdir().unwrap();
        init_blame_repo(tmp.path());
        let op = BlameFile::new(tmp.path(), "file.txt");
        let typed = op.run(&ctx()).await.unwrap();
        let value = op.execute(&ctx()).await.unwrap();
        assert_eq!(value["file"], "file.txt");
        assert_eq!(value["hunks"].as_array().unwrap().len(), typed.hunks.len());
    }
}
