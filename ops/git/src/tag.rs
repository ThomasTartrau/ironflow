//! Tag operations.

use std::path::PathBuf;

use async_trait::async_trait;
use git2::{Repository, Signature};
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::helpers::{blocking, to_value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagLightweightOutput {
    pub name: String,
    pub target: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagAnnotatedOutput {
    pub name: String,
    pub oid: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagDeleteOutput {
    pub name: String,
    pub oid: String,
    pub deleted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagListOutput {
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
}

/// Create a lightweight tag.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::tag::TagCreateLightweight;
/// use ironflow_core::operation::Operation;
///
/// let op = TagCreateLightweight::new("/path/to/repo", "v1.0.0");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct TagCreateLightweight {
    repo_path: PathBuf,
    name: String,
}

impl TagCreateLightweight {
    /// Create a new lightweight-tag operation.
    pub fn new(repo_path: impl Into<PathBuf>, name: impl Into<String>) -> Self {
        Self {
            repo_path: repo_path.into(),
            name: name.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(
        &self,
        _ctx: &OperationContext,
    ) -> Result<TagLightweightOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let name = self.name.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let head = repo.head()?.peel_to_commit()?;
            let obj = head.as_object();
            repo.tag_lightweight(&name, obj, false)?;
            Ok(TagLightweightOutput {
                name,
                target: head.id().to_string(),
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for TagCreateLightweight {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "repo_path": self.repo_path, "name": self.name }))
    }
}

impl TypedOperation for TagCreateLightweight {
    type Output = TagLightweightOutput;
}

/// Create an annotated tag.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::tag::TagCreateAnnotated;
/// use ironflow_core::operation::Operation;
///
/// let op = TagCreateAnnotated::new("/path/to/repo", "v1.0.0", "Release 1.0", "Alice", "alice@example.com");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct TagCreateAnnotated {
    repo_path: PathBuf,
    name: String,
    message: String,
    tagger_name: String,
    tagger_email: String,
}

impl TagCreateAnnotated {
    /// Create a new annotated-tag operation.
    pub fn new(
        repo_path: impl Into<PathBuf>,
        name: impl Into<String>,
        message: impl Into<String>,
        tagger_name: impl Into<String>,
        tagger_email: impl Into<String>,
    ) -> Self {
        Self {
            repo_path: repo_path.into(),
            name: name.into(),
            message: message.into(),
            tagger_name: tagger_name.into(),
            tagger_email: tagger_email.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<TagAnnotatedOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let name = self.name.clone();
        let message = self.message.clone();
        let tagger_name = self.tagger_name.clone();
        let tagger_email = self.tagger_email.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let head = repo.head()?.peel_to_commit()?;
            let obj = head.as_object();
            let sig = Signature::now(&tagger_name, &tagger_email)?;
            let oid = repo.tag(&name, obj, &sig, &message, false)?;
            Ok(TagAnnotatedOutput {
                name,
                oid: oid.to_string(),
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for TagCreateAnnotated {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(
            serde_json::json!({ "repo_path": self.repo_path, "name": self.name, "message": self.message }),
        )
    }
}

impl TypedOperation for TagCreateAnnotated {
    type Output = TagAnnotatedOutput;
}

/// Delete a tag.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::tag::TagDelete;
/// use ironflow_core::operation::Operation;
///
/// let op = TagDelete::new("/path/to/repo", "v1.0.0");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct TagDelete {
    repo_path: PathBuf,
    name: String,
}

impl TagDelete {
    /// Create a new tag-delete operation.
    pub fn new(repo_path: impl Into<PathBuf>, name: impl Into<String>) -> Self {
        Self {
            repo_path: repo_path.into(),
            name: name.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<TagDeleteOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let name = self.name.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let refname = format!("refs/tags/{name}");
            let oid = repo.refname_to_id(&refname)?;
            repo.tag_delete(&name)?;
            Ok(TagDeleteOutput {
                name,
                oid: oid.to_string(),
                deleted: true,
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for TagDelete {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "repo_path": self.repo_path, "name": self.name }))
    }
}

impl TypedOperation for TagDelete {
    type Output = TagDeleteOutput;
}

/// List all tags.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::tag::TagList;
/// use ironflow_core::operation::Operation;
///
/// let op = TagList::new("/path/to/repo");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct TagList {
    repo_path: PathBuf,
}

impl TagList {
    /// Create a new tag-list operation.
    pub fn new(repo_path: impl Into<PathBuf>) -> Self {
        Self {
            repo_path: repo_path.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<TagListOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let tags = repo.tag_names(None)?;
            let list: Vec<String> = tags.iter().filter_map(|t| t.map(String::from)).collect();
            Ok(TagListOutput {
                tags: list,
                pattern: None,
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for TagList {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "repo_path": self.repo_path }))
    }
}

impl TypedOperation for TagList {
    type Output = TagListOutput;
}

/// List tags matching a glob pattern.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::tag::TagListMatch;
/// use ironflow_core::operation::Operation;
///
/// let op = TagListMatch::new("/path/to/repo", "v1.*");
/// assert_eq!(op.kind(), "git");
/// ```
pub struct TagListMatch {
    repo_path: PathBuf,
    pattern: String,
}

impl TagListMatch {
    /// Create a new tag-list-match operation.
    pub fn new(repo_path: impl Into<PathBuf>, pattern: impl Into<String>) -> Self {
        Self {
            repo_path: repo_path.into(),
            pattern: pattern.into(),
        }
    }

    /// Execute and return a typed result.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<TagListOutput, OperationError> {
        let repo_path = self.repo_path.clone();
        let pattern = self.pattern.clone();
        blocking(move || {
            let repo = Repository::open(&repo_path)?;
            let tags = repo.tag_names(Some(&pattern))?;
            let list: Vec<String> = tags.iter().filter_map(|t| t.map(String::from)).collect();
            Ok(TagListOutput {
                tags: list,
                pattern: Some(pattern),
            })
        })
        .await
    }
}

#[async_trait]
impl Operation for TagListMatch {
    fn kind(&self) -> &str {
        "git"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "repo_path": self.repo_path, "pattern": self.pattern }))
    }
}

impl TypedOperation for TagListMatch {
    type Output = TagListOutput;
}

#[cfg(test)]
mod tests {
    use ironflow_core::operation::Operation;

    use super::*;
    use crate::test_helpers::{ctx, init_repo};

    #[tokio::test]
    async fn annotated_tag_create_and_list() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        let result = TagCreateAnnotated::new(tmp.path(), "v2.0", "Release", "A", "a@t.com")
            .run(&ctx())
            .await
            .unwrap();
        assert_eq!(result.name, "v2.0");
        assert!(!result.oid.is_empty());
        let list = TagList::new(tmp.path()).run(&ctx()).await.unwrap();
        assert!(list.tags.contains(&"v2.0".to_string()));
    }

    #[tokio::test]
    async fn delete_tag() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        TagCreateLightweight::new(tmp.path(), "v1.0")
            .run(&ctx())
            .await
            .unwrap();
        let result = TagDelete::new(tmp.path(), "v1.0")
            .run(&ctx())
            .await
            .unwrap();
        assert!(result.deleted);
        let list = TagList::new(tmp.path()).run(&ctx()).await.unwrap();
        assert!(!list.tags.contains(&"v1.0".to_string()));
    }

    #[tokio::test]
    async fn list_match_filters() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        TagCreateLightweight::new(tmp.path(), "v1.0")
            .run(&ctx())
            .await
            .unwrap();
        TagCreateLightweight::new(tmp.path(), "release-1")
            .run(&ctx())
            .await
            .unwrap();
        let result = TagListMatch::new(tmp.path(), "v*")
            .run(&ctx())
            .await
            .unwrap();
        assert!(result.tags.contains(&"v1.0".to_string()));
        assert!(!result.tags.contains(&"release-1".to_string()));
        assert_eq!(result.pattern.as_deref(), Some("v*"));
    }

    #[tokio::test]
    async fn delete_nonexistent_tag_fails() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        assert!(
            TagDelete::new(tmp.path(), "nope")
                .run(&ctx())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn execute_serializes_correctly() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        let value = TagList::new(tmp.path()).execute(&ctx()).await.unwrap();
        assert!(value["tags"].is_array());
    }
}
