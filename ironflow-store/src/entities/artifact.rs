//! [`Artifact`] entity -- metadata for a file produced by a step.
//!
//! The bytes themselves live in a blob store (`ironflow-artifacts`). This
//! metadata is the source of truth: a blob with no [`Artifact`] row is never
//! listed and never served.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A file produced by a step and persisted for later steps or download.
///
/// # Examples
///
/// ```
/// use chrono::Utc;
/// use ironflow_store::entities::Artifact;
/// use uuid::Uuid;
///
/// let artifact = Artifact {
///     id: Uuid::now_v7(),
///     run_id: Uuid::now_v7(),
///     step_id: Uuid::now_v7(),
///     name: "report.html".to_string(),
///     storage_key: "artifacts/run/step/id".to_string(),
///     content_type: "text/html".to_string(),
///     size_bytes: 142,
///     sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_string(),
///     created_at: Utc::now(),
///     updated_at: Utc::now(),
/// };
/// assert_eq!(artifact.name, "report.html");
/// ```
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    /// Unique artifact identifier (UUIDv7).
    pub id: Uuid,
    /// The run this artifact belongs to.
    pub run_id: Uuid,
    /// The step that produced it.
    pub step_id: Uuid,
    /// User-facing file name, unique within the step.
    pub name: String,
    /// Key under which the bytes are stored in the blob store.
    ///
    /// Derived from UUIDs only, never from [`name`](Artifact::name).
    pub storage_key: String,
    /// MIME type served on download.
    pub content_type: String,
    /// Size of the stored content, in bytes.
    pub size_bytes: u64,
    /// Lowercase hex SHA-256 of the content.
    pub sha256: String,
    /// When the artifact was recorded.
    pub created_at: DateTime<Utc>,
    /// When the record was last updated.
    pub updated_at: DateTime<Utc>,
}

/// Request to record a new artifact.
///
/// Created by the engine after the bytes have been written to the blob store:
/// the blob is written first, so a crash in between leaves an unreferenced blob
/// rather than a row pointing at nothing.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::NewArtifact;
/// use uuid::Uuid;
///
/// let req = NewArtifact {
///     id: Uuid::now_v7(),
///     run_id: Uuid::now_v7(),
///     step_id: Uuid::now_v7(),
///     name: "build.log".to_string(),
///     storage_key: "artifacts/run/step/id".to_string(),
///     content_type: "text/plain".to_string(),
///     size_bytes: 2048,
///     sha256: "0".repeat(64),
/// };
/// assert_eq!(req.size_bytes, 2048);
/// ```
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewArtifact {
    /// Identifier to assign; also the last segment of the storage key.
    pub id: Uuid,
    /// The run this artifact belongs to.
    pub run_id: Uuid,
    /// The step that produced it.
    pub step_id: Uuid,
    /// User-facing file name, unique within the step.
    pub name: String,
    /// Key under which the bytes were stored.
    pub storage_key: String,
    /// MIME type to serve on download.
    pub content_type: String,
    /// Size of the stored content, in bytes.
    pub size_bytes: u64,
    /// Lowercase hex SHA-256 of the content.
    pub sha256: String,
}

/// Which artifact a step wants to consume, by producing step name.
///
/// Resolved within the current run and attempt, among steps positioned strictly
/// before the consumer. When several steps share the same name, the one closest
/// to the consumer wins.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::ArtifactLookup;
/// use uuid::Uuid;
///
/// let lookup = ArtifactLookup {
///     run_id: Uuid::now_v7(),
///     attempt: 1,
///     before_position: 3,
///     step_name: "build".to_string(),
///     name: "report.html".to_string(),
/// };
/// assert_eq!(lookup.before_position, 3);
/// ```
#[derive(Debug, Clone)]
pub struct ArtifactLookup {
    /// Run to search in.
    pub run_id: Uuid,
    /// Attempt to search in.
    pub attempt: u32,
    /// Only consider steps at a strictly lower position.
    pub before_position: u32,
    /// Name of the producing step.
    pub step_name: String,
    /// Name of the artifact.
    pub name: String,
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
            storage_key: "artifacts/a/b/c".to_string(),
            content_type: "text/html".to_string(),
            size_bytes: 142,
            sha256: "0".repeat(64),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn artifact_serde_roundtrips() {
        let artifact = sample();
        let json = serde_json::to_string(&artifact).expect("serialize");
        let parsed: Artifact = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(parsed.name, artifact.name);
        assert_eq!(parsed.sha256, artifact.sha256);
        assert_eq!(parsed.size_bytes, artifact.size_bytes);
    }

    #[test]
    fn artifact_exposes_the_storage_key() {
        // The key is internal plumbing but stays serialized: the API layer maps
        // to its own DTO and decides what to expose.
        let json = serde_json::to_string(&sample()).expect("serialize");
        assert!(json.contains("storage_key"));
    }

    #[test]
    fn new_artifact_serde_roundtrips() {
        let req = NewArtifact {
            id: Uuid::now_v7(),
            run_id: Uuid::now_v7(),
            step_id: Uuid::now_v7(),
            name: "build.log".to_string(),
            storage_key: "artifacts/a/b/c".to_string(),
            content_type: "text/plain".to_string(),
            size_bytes: 0,
            sha256: "0".repeat(64),
        };

        let json = serde_json::to_string(&req).expect("serialize");
        let parsed: NewArtifact = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(parsed.name, "build.log");
        assert_eq!(parsed.size_bytes, 0);
    }
}
