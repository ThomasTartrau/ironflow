//! Artifact DTO -- public API representation of a file produced by a step.

use chrono::{DateTime, Utc};
use ironflow_store::models::Artifact;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// An artifact as exposed by the REST API.
///
/// The storage key is deliberately absent: it is internal plumbing, and callers
/// address an artifact by `(run, step, name)` through the download route.
///
/// # Examples
///
/// ```
/// use chrono::Utc;
/// use ironflow_api::entities::ArtifactResponse;
/// use uuid::Uuid;
///
/// let response = ArtifactResponse {
///     id: Uuid::now_v7(),
///     step_id: Uuid::now_v7(),
///     name: "report.html".to_string(),
///     content_type: "text/html".to_string(),
///     size_bytes: 142,
///     sha256: "0".repeat(64),
///     created_at: Utc::now(),
/// };
/// assert_eq!(response.name, "report.html");
/// ```
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactResponse {
    /// Unique artifact identifier.
    pub id: Uuid,
    /// The step that produced it.
    pub step_id: Uuid,
    /// File name, unique within the step.
    pub name: String,
    /// MIME type served on download.
    pub content_type: String,
    /// Size of the content, in bytes.
    pub size_bytes: u64,
    /// Lowercase hex SHA-256 of the content.
    pub sha256: String,
    /// When the artifact was recorded.
    pub created_at: DateTime<Utc>,
}

impl From<Artifact> for ArtifactResponse {
    fn from(artifact: Artifact) -> Self {
        Self {
            id: artifact.id,
            step_id: artifact.step_id,
            name: artifact.name,
            content_type: artifact.content_type,
            size_bytes: artifact.size_bytes,
            sha256: artifact.sha256,
            created_at: artifact.created_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Artifact {
        Artifact {
            id: Uuid::now_v7(),
            run_id: Uuid::now_v7(),
            step_id: Uuid::now_v7(),
            name: "report.html".to_string(),
            storage_key: "artifacts/secret/path/uuid".to_string(),
            content_type: "text/html".to_string(),
            size_bytes: 142,
            sha256: "0".repeat(64),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn conversion_keeps_the_caller_facing_fields() {
        let artifact = sample();
        let response = ArtifactResponse::from(artifact.clone());

        assert_eq!(response.id, artifact.id);
        assert_eq!(response.step_id, artifact.step_id);
        assert_eq!(response.name, artifact.name);
        assert_eq!(response.size_bytes, artifact.size_bytes);
        assert_eq!(response.sha256, artifact.sha256);
    }

    #[test]
    fn serialization_never_leaks_the_storage_key() {
        let json = serde_json::to_string(&ArtifactResponse::from(sample())).expect("serialize");

        assert!(!json.contains("storage_key"));
        assert!(!json.contains("secret/path"));
    }

    #[test]
    fn serialization_omits_the_run_id_carried_by_the_route() {
        let json = serde_json::to_string(&ArtifactResponse::from(sample())).expect("serialize");
        assert!(!json.contains("run_id"));
    }
}
