//! The [`ArtifactStore`] trait -- metadata for files produced by steps.
//!
//! This trait owns the *records*; the bytes live in a blob store
//! (`ironflow-artifacts`). A blob with no record here is unreachable, which is
//! what makes "write the blob, then the record" a safe ordering.
//!
//! Built-in implementations:
//!
//! - [`InMemoryStore`](crate::memory::InMemoryStore) -- development and testing.
//! - `PostgresStore` -- production (behind the `store-postgres` feature).

use uuid::Uuid;

use crate::entities::{Artifact, ArtifactLookup, NewArtifact};
use crate::store::StoreFuture;

/// Async storage abstraction for artifact metadata.
///
/// All methods return a [`StoreFuture`] for object safety, so the store can be
/// used as `Arc<dyn ArtifactStore>`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_store::prelude::*;
/// use uuid::Uuid;
///
/// # async fn example(store: &dyn ArtifactStore, run_id: Uuid, step_id: Uuid)
/// # -> Result<(), ironflow_store::error::StoreError> {
/// let artifacts = store.list_artifacts_for_run(run_id).await?;
/// for artifact in &artifacts {
///     println!("{} ({} bytes)", artifact.name, artifact.size_bytes);
/// }
///
/// let one = store.get_artifact(step_id, "report.html").await?;
/// assert!(one.is_none() || one.unwrap().name == "report.html");
/// # Ok(())
/// # }
/// ```
pub trait ArtifactStore: Send + Sync {
    /// Record a new artifact.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::StepNotFound`](crate::error::StoreError::StepNotFound)
    /// when the owning step does not exist, and
    /// [`StoreError::DuplicateArtifact`](crate::error::StoreError::DuplicateArtifact)
    /// when the step already has an artifact with this name.
    fn create_artifact(&self, artifact: NewArtifact) -> StoreFuture<'_, Artifact>;

    /// Look up one artifact of a step by name.
    ///
    /// Returns `None` when the step has no artifact with that name.
    fn get_artifact(&self, step_id: Uuid, name: &str) -> StoreFuture<'_, Option<Artifact>>;

    /// List every artifact of a run, across all steps and attempts.
    ///
    /// A single call serves a whole run detail page: callers group by
    /// [`step_id`](Artifact::step_id) rather than querying per step.
    /// Ordered by step then name.
    fn list_artifacts_for_run(&self, run_id: Uuid) -> StoreFuture<'_, Vec<Artifact>>;

    /// Resolve the artifact a step declared as an input.
    ///
    /// Searches the run and attempt named by `lookup`, among steps whose
    /// position is strictly below `before_position`, and returns the match
    /// produced by the step closest to the consumer. Returns `None` when
    /// nothing matches.
    fn find_artifact_for_input(&self, lookup: ArtifactLookup) -> StoreFuture<'_, Option<Artifact>>;
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use crate::entities::{ArtifactLookup, NewArtifact};

    #[test]
    fn new_artifact_carries_its_own_id() {
        // The id is chosen by the caller because it is also the last segment of
        // the storage key, which is built before the record is inserted.
        let id = Uuid::now_v7();
        let req = NewArtifact {
            id,
            run_id: Uuid::now_v7(),
            step_id: Uuid::now_v7(),
            name: "a.txt".to_string(),
            storage_key: format!("artifacts/x/y/{id}"),
            content_type: "text/plain".to_string(),
            size_bytes: 1,
            sha256: "0".repeat(64),
        };

        assert!(req.storage_key.ends_with(&id.to_string()));
    }

    #[test]
    fn lookup_is_scoped_to_a_run_and_attempt() {
        let lookup = ArtifactLookup {
            run_id: Uuid::now_v7(),
            attempt: 2,
            before_position: 0,
            step_name: "build".to_string(),
            name: "a.txt".to_string(),
        };

        assert_eq!(lookup.attempt, 2);
        assert_eq!(lookup.before_position, 0);
    }
}
