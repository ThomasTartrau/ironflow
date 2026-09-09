//! Image cleanup operations: prune.

use std::collections::HashMap;

use async_trait::async_trait;
use bollard::Docker;
use bollard::query_parameters::PruneImagesOptions;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::containers::DockerRef;
use crate::helpers::{docker_error, to_value};

// ---------------------------------------------------------------------------
// ImagePrune
// ---------------------------------------------------------------------------

/// Output of pruning unused images.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImagePruneOutput {
    /// Deleted image IDs.
    pub images_deleted: Vec<String>,
    /// Disk space reclaimed in bytes.
    pub space_reclaimed: u64,
}

/// Remove unused images.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_docker::images::ImagePrune;
/// use ironflow_ops_docker::DockerClient;
/// use ironflow_core::operation::Operation;
///
/// let client = DockerClient::connect_local().unwrap();
/// let op = ImagePrune::new(&client);
/// assert_eq!(op.kind(), "docker");
/// ```
pub struct ImagePrune {
    docker: Docker,
    filters: HashMap<String, Vec<String>>,
}

impl ImagePrune {
    /// Create a new image-prune operation.
    pub fn new(client: impl Into<DockerRef>) -> Self {
        Self {
            docker: client.into().0,
            filters: HashMap::new(),
        }
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
    pub async fn run(&self, _ctx: &OperationContext) -> Result<ImagePruneOutput, OperationError> {
        let options = PruneImagesOptions {
            filters: Some(self.filters.clone()),
        };
        let response = self
            .docker
            .prune_images(Some(options))
            .await
            .map_err(docker_error)?;
        let images_deleted = response
            .images_deleted
            .unwrap_or_default()
            .into_iter()
            .filter_map(|i| i.deleted.or(i.untagged))
            .collect();
        Ok(ImagePruneOutput {
            images_deleted,
            space_reclaimed: response.space_reclaimed.unwrap_or(0) as u64,
        })
    }
}

#[async_trait]
impl Operation for ImagePrune {
    fn kind(&self) -> &str {
        "docker"
    }

    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }

    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "operation": "image_prune",
        }))
    }
}

impl TypedOperation for ImagePrune {
    type Output = ImagePruneOutput;
}
