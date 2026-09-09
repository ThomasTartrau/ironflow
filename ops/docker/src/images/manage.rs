//! Image management: list, pull, push, build, inspect, remove.

use std::collections::HashMap;

use async_trait::async_trait;
use bollard::Docker;
use bollard::query_parameters::{
    CreateImageOptions, ListImagesOptions, PushImageOptions, RemoveImageOptions,
};
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_stream::StreamExt;

use crate::containers::DockerRef;
use crate::helpers::{docker_error, to_value};

// ---------------------------------------------------------------------------
// ImageList
// ---------------------------------------------------------------------------

/// A single entry in the image list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageListEntry {
    /// Image ID.
    pub id: String,
    /// Repository tags.
    pub repo_tags: Vec<String>,
    /// Image size in bytes.
    pub size: i64,
    /// Creation timestamp.
    pub created: i64,
}

/// Output of an image list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageListOutput {
    /// The images.
    pub images: Vec<ImageListEntry>,
}

/// List local images.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_docker::images::ImageList;
/// use ironflow_ops_docker::DockerClient;
/// use ironflow_core::operation::Operation;
///
/// let client = DockerClient::connect_local().unwrap();
/// let op = ImageList::new(&client);
/// assert_eq!(op.kind(), "docker");
/// ```
pub struct ImageList {
    docker: Docker,
    all: bool,
    filters: HashMap<String, Vec<String>>,
}

impl ImageList {
    /// Create a new image-list operation.
    pub fn new(client: impl Into<DockerRef>) -> Self {
        Self {
            docker: client.into().0,
            all: false,
            filters: HashMap::new(),
        }
    }

    /// Include intermediate images.
    pub fn all(mut self) -> Self {
        self.all = true;
        self
    }

    /// Add a filter.
    pub fn filter(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.filters
            .entry(key.into())
            .or_default()
            .push(value.into());
        self
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] if the Docker daemon is
    /// unreachable.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<ImageListOutput, OperationError> {
        let options = ListImagesOptions {
            all: self.all,
            filters: Some(self.filters.clone()),
            ..Default::default()
        };
        let images = self
            .docker
            .list_images(Some(options))
            .await
            .map_err(docker_error)?;
        let entries = images
            .into_iter()
            .map(|i| ImageListEntry {
                id: i.id,
                repo_tags: i.repo_tags,
                size: i.size,
                created: i.created,
            })
            .collect();
        Ok(ImageListOutput { images: entries })
    }
}

#[async_trait]
impl Operation for ImageList {
    fn kind(&self) -> &str {
        "docker"
    }

    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "operation": "image_list",
            "all": self.all,
        }))
    }
}

impl TypedOperation for ImageList {
    type Output = ImageListOutput;
}

// ---------------------------------------------------------------------------
// ImagePull
// ---------------------------------------------------------------------------

/// Output of an image pull.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImagePullOutput {
    /// The pulled image reference.
    pub image: String,
}

/// Pull an image from a registry.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_docker::images::ImagePull;
/// use ironflow_ops_docker::DockerClient;
/// use ironflow_core::operation::Operation;
///
/// let client = DockerClient::connect_local().unwrap();
/// let op = ImagePull::new(&client, "alpine:latest");
/// assert_eq!(op.kind(), "docker");
/// ```
pub struct ImagePull {
    docker: Docker,
    image: String,
}

impl ImagePull {
    /// Create a new image-pull operation.
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
    /// Returns [`OperationError::External`] if the image cannot be pulled.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<ImagePullOutput, OperationError> {
        let options = CreateImageOptions {
            from_image: Some(self.image.clone()),
            ..Default::default()
        };
        let mut stream = self.docker.create_image(Some(options), None, None);
        while let Some(result) = stream.next().await {
            result.map_err(docker_error)?;
        }
        Ok(ImagePullOutput {
            image: self.image.clone(),
        })
    }
}

#[async_trait]
impl Operation for ImagePull {
    fn kind(&self) -> &str {
        "docker"
    }

    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "operation": "image_pull",
            "image": self.image,
        }))
    }
}

impl TypedOperation for ImagePull {
    type Output = ImagePullOutput;
}

// ---------------------------------------------------------------------------
// ImagePush
// ---------------------------------------------------------------------------

/// Output of an image push.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImagePushOutput {
    /// The pushed image reference.
    pub image: String,
}

/// Push an image to a registry.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_docker::images::ImagePush;
/// use ironflow_ops_docker::DockerClient;
/// use ironflow_core::operation::Operation;
///
/// let client = DockerClient::connect_local().unwrap();
/// let op = ImagePush::new(&client, "my-image:latest");
/// assert_eq!(op.kind(), "docker");
/// ```
pub struct ImagePush {
    docker: Docker,
    image: String,
    tag: Option<String>,
}

impl ImagePush {
    /// Create a new image-push operation.
    pub fn new(client: impl Into<DockerRef>, image: impl Into<String>) -> Self {
        Self {
            docker: client.into().0,
            image: image.into(),
            tag: None,
        }
    }

    /// Set the tag to push.
    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.tag = Some(tag.into());
        self
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] if the push fails.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<ImagePushOutput, OperationError> {
        let options = PushImageOptions {
            tag: Some(self.tag.clone().unwrap_or_else(|| "latest".to_string())),
            platform: None,
        };
        let mut stream = self.docker.push_image(&self.image, Some(options), None);
        while let Some(result) = stream.next().await {
            result.map_err(docker_error)?;
        }
        Ok(ImagePushOutput {
            image: self.image.clone(),
        })
    }
}

#[async_trait]
impl Operation for ImagePush {
    fn kind(&self) -> &str {
        "docker"
    }

    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "operation": "image_push",
            "image": self.image,
        }))
    }
}

impl TypedOperation for ImagePush {
    type Output = ImagePushOutput;
}

// ---------------------------------------------------------------------------
// ImageInspect
// ---------------------------------------------------------------------------

/// Output of an image inspection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageInspectOutput {
    /// The full inspection response as JSON.
    pub data: Value,
}

/// Inspect an image.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_docker::images::ImageInspect;
/// use ironflow_ops_docker::DockerClient;
/// use ironflow_core::operation::Operation;
///
/// let client = DockerClient::connect_local().unwrap();
/// let op = ImageInspect::new(&client, "alpine:latest");
/// assert_eq!(op.kind(), "docker");
/// ```
pub struct ImageInspect {
    docker: Docker,
    image: String,
}

impl ImageInspect {
    /// Create a new image-inspect operation.
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
    pub async fn run(&self, _ctx: &OperationContext) -> Result<ImageInspectOutput, OperationError> {
        let response = self
            .docker
            .inspect_image(&self.image)
            .await
            .map_err(docker_error)?;
        let data = serde_json::to_value(&response).map_err(|e| OperationError::External {
            origin: "docker".to_string(),
            message: e.to_string(),
        })?;
        Ok(ImageInspectOutput { data })
    }
}

#[async_trait]
impl Operation for ImageInspect {
    fn kind(&self) -> &str {
        "docker"
    }

    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "operation": "image_inspect",
            "image": self.image,
        }))
    }
}

impl TypedOperation for ImageInspect {
    type Output = ImageInspectOutput;
}

// ---------------------------------------------------------------------------
// ImageRemove
// ---------------------------------------------------------------------------

/// Output of an image removal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageRemoveOutput {
    /// The removed image reference.
    pub image: String,
}

/// Remove an image.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_docker::images::ImageRemove;
/// use ironflow_ops_docker::DockerClient;
/// use ironflow_core::operation::Operation;
///
/// let client = DockerClient::connect_local().unwrap();
/// let op = ImageRemove::new(&client, "alpine:latest");
/// assert_eq!(op.kind(), "docker");
/// ```
pub struct ImageRemove {
    docker: Docker,
    image: String,
    force: bool,
    no_prune: bool,
}

impl ImageRemove {
    /// Create a new image-remove operation.
    pub fn new(client: impl Into<DockerRef>, image: impl Into<String>) -> Self {
        Self {
            docker: client.into().0,
            image: image.into(),
            force: false,
            no_prune: false,
        }
    }

    /// Force-remove the image.
    pub fn force(mut self) -> Self {
        self.force = true;
        self
    }

    /// Do not delete untagged parent images.
    pub fn no_prune(mut self) -> Self {
        self.no_prune = true;
        self
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] if the image does not exist.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<ImageRemoveOutput, OperationError> {
        let options = RemoveImageOptions {
            force: self.force,
            noprune: self.no_prune,
            platforms: None,
        };
        self.docker
            .remove_image(&self.image, Some(options), None)
            .await
            .map_err(docker_error)?;
        Ok(ImageRemoveOutput {
            image: self.image.clone(),
        })
    }
}

#[async_trait]
impl Operation for ImageRemove {
    fn kind(&self) -> &str {
        "docker"
    }

    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "operation": "image_remove",
            "image": self.image,
        }))
    }
}

impl TypedOperation for ImageRemove {
    type Output = ImageRemoveOutput;
}
