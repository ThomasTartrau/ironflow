//! Image tag, history, and search operations.

use async_trait::async_trait;
use bollard::Docker;
use bollard::query_parameters::{SearchImagesOptions, TagImageOptions};
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::containers::DockerRef;
use crate::helpers::{docker_error, to_value};

// ---------------------------------------------------------------------------
// ImageTag
// ---------------------------------------------------------------------------

/// Output of an image tag.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageTagOutput {
    /// The source image.
    pub source: String,
    /// The new repository name.
    pub repo: String,
    /// The new tag.
    pub tag: String,
}

/// Add a tag to an image.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_docker::images::ImageTag;
/// use ironflow_ops_docker::DockerClient;
/// use ironflow_core::operation::Operation;
///
/// let client = DockerClient::connect_local().unwrap();
/// let op = ImageTag::new(&client, "alpine:latest", "my-repo", "v1");
/// assert_eq!(op.kind(), "docker");
/// ```
pub struct ImageTag {
    docker: Docker,
    source: String,
    repo: String,
    tag: String,
}

impl ImageTag {
    /// Create a new image-tag operation.
    pub fn new(
        client: impl Into<DockerRef>,
        source: impl Into<String>,
        repo: impl Into<String>,
        tag: impl Into<String>,
    ) -> Self {
        Self {
            docker: client.into().0,
            source: source.into(),
            repo: repo.into(),
            tag: tag.into(),
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] if the source image does not exist.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<ImageTagOutput, OperationError> {
        let options = TagImageOptions {
            repo: Some(self.repo.clone()),
            tag: Some(self.tag.clone()),
        };
        self.docker
            .tag_image(&self.source, Some(options))
            .await
            .map_err(docker_error)?;
        Ok(ImageTagOutput {
            source: self.source.clone(),
            repo: self.repo.clone(),
            tag: self.tag.clone(),
        })
    }
}

#[async_trait]
impl Operation for ImageTag {
    fn kind(&self) -> &str {
        "docker"
    }

    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "operation": "image_tag",
            "source": self.source,
            "repo": self.repo,
            "tag": self.tag,
        }))
    }
}

impl TypedOperation for ImageTag {
    type Output = ImageTagOutput;
}

// ---------------------------------------------------------------------------
// ImageHistory
// ---------------------------------------------------------------------------

/// A single layer in the image history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageHistoryEntry {
    /// Layer ID.
    pub id: String,
    /// Created timestamp.
    pub created: i64,
    /// Created-by command.
    pub created_by: String,
    /// Layer size in bytes.
    pub size: i64,
}

/// Output of an image history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageHistoryOutput {
    /// The layers.
    pub layers: Vec<ImageHistoryEntry>,
}

/// Get the history of an image (layers).
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_docker::images::ImageHistory;
/// use ironflow_ops_docker::DockerClient;
/// use ironflow_core::operation::Operation;
///
/// let client = DockerClient::connect_local().unwrap();
/// let op = ImageHistory::new(&client, "alpine:latest");
/// assert_eq!(op.kind(), "docker");
/// ```
pub struct ImageHistory {
    docker: Docker,
    image: String,
}

impl ImageHistory {
    /// Create a new image-history operation.
    pub fn new(client: impl Into<DockerRef>, image: impl Into<String>) -> Self {
        Self {
            docker: client.into().0,
            image: image.into(),
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] if the image does not exist.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<ImageHistoryOutput, OperationError> {
        let history = self
            .docker
            .image_history(&self.image)
            .await
            .map_err(docker_error)?;
        let layers = history
            .into_iter()
            .map(|h| ImageHistoryEntry {
                id: h.id,
                created: h.created,
                created_by: h.created_by,
                size: h.size,
            })
            .collect();
        Ok(ImageHistoryOutput { layers })
    }
}

#[async_trait]
impl Operation for ImageHistory {
    fn kind(&self) -> &str {
        "docker"
    }

    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "operation": "image_history",
            "image": self.image,
        }))
    }
}

impl TypedOperation for ImageHistory {
    type Output = ImageHistoryOutput;
}

// ---------------------------------------------------------------------------
// ImageSearch
// ---------------------------------------------------------------------------

/// A single search result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageSearchEntry {
    /// Image name.
    pub name: String,
    /// Description.
    pub description: String,
    /// Star count.
    pub star_count: i64,
    /// Whether it is an official image.
    pub is_official: bool,
}

/// Output of an image search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageSearchOutput {
    /// The search results.
    pub results: Vec<ImageSearchEntry>,
}

/// Search for images on the registry.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_docker::images::ImageSearch;
/// use ironflow_ops_docker::DockerClient;
/// use ironflow_core::operation::Operation;
///
/// let client = DockerClient::connect_local().unwrap();
/// let op = ImageSearch::new(&client, "alpine");
/// assert_eq!(op.kind(), "docker");
/// ```
pub struct ImageSearch {
    docker: Docker,
    term: String,
    limit: Option<i64>,
}

impl ImageSearch {
    /// Create a new image-search operation.
    pub fn new(client: impl Into<DockerRef>, term: impl Into<String>) -> Self {
        Self {
            docker: client.into().0,
            term: term.into(),
            limit: None,
        }
    }

    /// Set the maximum number of results.
    pub fn limit(mut self, limit: i64) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] if the search fails.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<ImageSearchOutput, OperationError> {
        let options = SearchImagesOptions {
            term: self.term.clone(),
            limit: self.limit.map(|l| l as i32),
            ..Default::default()
        };
        let results = self
            .docker
            .search_images(options)
            .await
            .map_err(docker_error)?;
        let entries = results
            .into_iter()
            .map(|r| ImageSearchEntry {
                name: r.name.unwrap_or_default(),
                description: r.description.unwrap_or_default(),
                star_count: r.star_count.unwrap_or(0),
                is_official: r.is_official.unwrap_or(false),
            })
            .collect();
        Ok(ImageSearchOutput { results: entries })
    }
}

#[async_trait]
impl Operation for ImageSearch {
    fn kind(&self) -> &str {
        "docker"
    }

    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "operation": "image_search",
            "term": self.term,
        }))
    }
}

impl TypedOperation for ImageSearch {
    type Output = ImageSearchOutput;
}
