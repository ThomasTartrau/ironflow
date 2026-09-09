//! Reset operations.

use std::path::PathBuf;

use async_trait::async_trait;
use git2::{Oid, Repository, ResetType};
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::helpers::{blocking, to_value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResetOutput {
    pub target: String,
    #[serde(rename = "type")]
    pub reset_type: String,
}

/// Reset HEAD to a target commit.
///
/// Three convenience constructors select the reset mode:
/// - [`Reset::soft`] -- move HEAD only (keep index and working directory)
/// - [`Reset::mixed`] -- move HEAD and reset index (keep working directory)
/// - [`Reset::hard`] -- move HEAD, reset index and working directory
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::reset::Reset;
/// use ironflow_core::operation::Operation;
///
/// let soft = Reset::soft("/path/to/repo", "abc123");
/// let mixed = Reset::mixed("/path/to/repo", "abc123");
/// let hard = Reset::hard("/path/to/repo", "abc123");
/// assert_eq!(soft.kind(), "git");
/// ```
pub struct Reset {
    repo_path: PathBuf,
    target: String,
    reset_type: ResetType,
}

impl Reset {
    /// Soft reset (move HEAD, keep index and working directory).
    pub fn soft(repo_path: impl Into<PathBuf>, target: impl Into<String>) -> Self {
        Self {
            repo_path: repo_path.into(),
            target: target.into(),
            reset_type: ResetType::Soft,
        }
    }

    /// Mixed reset (move HEAD and reset index, keep working directory).
    pub fn mixed(repo_path: impl Into<PathBuf>, target: impl Into<String>) -> Self {
        Self {
            repo_path: repo_path.into(),
            target: target.into(),
            reset_type: ResetType::Mixed,
        }
    }

    /// Hard reset (move HEAD, reset index and working directory).
    pub fn hard(repo_path: impl Into<PathBuf>, target: impl Into<String>) -> Self {
        Self {
            repo_path: repo_path.into(),
            target: target.into(),
            reset_type: ResetType::Hard,
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<ResetOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let target = self.target.clone();
        let reset_type = self.reset_type;
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let oid = Oid::from_str(&target)?;
            let obj = repo.find_object(oid, None)?;
            repo.reset(&obj, reset_type, None)?;
            Ok(ResetOutput {
                target,
                reset_type: reset_type_label(reset_type).to_string(),
            })
        })
        .await
    }
}

fn reset_type_label(rt: ResetType) -> &'static str {
    match rt {
        ResetType::Soft => "soft",
        ResetType::Mixed => "mixed",
        ResetType::Hard => "hard",
    }
}

#[async_trait]
impl Operation for Reset {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "repo_path": self.repo_path,
            "target": self.target,
            "type": reset_type_label(self.reset_type),
        }))
    }
}

impl TypedOperation for Reset {
    type Output = ResetOutput;
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use git2::{Repository, Signature};
    use ironflow_core::operation::Operation;

    use super::*;
    use crate::test_helpers::ctx;

    fn init_two_commits(path: &Path) -> String {
        let repo = Repository::init(path).unwrap();
        let sig = Signature::now("Test", "test@test.com").unwrap();
        fs::write(path.join("file.txt"), "v1").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("file.txt")).unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let c1 = repo
            .commit(Some("HEAD"), &sig, &sig, "first", &tree, &[])
            .unwrap();
        let parent = repo.find_commit(c1).unwrap();

        fs::write(path.join("file.txt"), "v2").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("file.txt")).unwrap();
        index.write().unwrap();
        let tree2 = repo.find_tree(index.write_tree().unwrap()).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "second", &tree2, &[&parent])
            .unwrap();
        c1.to_string()
    }

    #[tokio::test]
    async fn soft_reset_moves_head() {
        let tmp = tempfile::tempdir().unwrap();
        let first_oid = init_two_commits(tmp.path());
        let result = Reset::soft(tmp.path(), &first_oid)
            .run(&ctx())
            .await
            .unwrap();
        assert_eq!(result.target, first_oid);
        assert_eq!(result.reset_type, "soft");
        let repo = Repository::open(tmp.path()).unwrap();
        assert_eq!(
            repo.head()
                .unwrap()
                .peel_to_commit()
                .unwrap()
                .id()
                .to_string(),
            first_oid
        );
        assert_eq!(
            fs::read_to_string(tmp.path().join("file.txt")).unwrap(),
            "v2"
        );
    }

    #[tokio::test]
    async fn hard_reset_restores_workdir() {
        let tmp = tempfile::tempdir().unwrap();
        let first_oid = init_two_commits(tmp.path());
        let result = Reset::hard(tmp.path(), &first_oid)
            .run(&ctx())
            .await
            .unwrap();
        assert_eq!(result.reset_type, "hard");
        assert_eq!(
            fs::read_to_string(tmp.path().join("file.txt")).unwrap(),
            "v1"
        );
    }

    #[tokio::test]
    async fn reset_invalid_target_fails() {
        let tmp = tempfile::tempdir().unwrap();
        init_two_commits(tmp.path());
        assert!(
            Reset::soft(tmp.path(), "0000000000000000000000000000000000000000")
                .run(&ctx())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn execute_serializes_correctly() {
        let tmp = tempfile::tempdir().unwrap();
        let first_oid = init_two_commits(tmp.path());
        let op = Reset::mixed(tmp.path(), &first_oid);
        let value = op.execute(&ctx()).await.unwrap();
        assert_eq!(value["type"], "mixed");
        assert_eq!(value["target"], first_oid);
    }
}
