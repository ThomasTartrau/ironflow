//! [`ArtifactStore`] implementation for [`PostgresStore`].

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::artifact_store::ArtifactStore;
use crate::entities::{Artifact, ArtifactLookup, NewArtifact};
use crate::error::StoreError;
use crate::store::StoreFuture;

use super::PostgresStore;

/// Postgres error code for a unique constraint violation.
const UNIQUE_VIOLATION: &str = "23505";
/// Postgres error code for a foreign key violation.
const FOREIGN_KEY_VIOLATION: &str = "23503";

/// Intermediate row struct matching the `ironflow.step_artifacts` columns.
struct ArtifactRow {
    id: Uuid,
    run_id: Uuid,
    step_id: Uuid,
    name: String,
    storage_key: String,
    content_type: String,
    size_bytes: i64,
    sha256: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<ArtifactRow> for Artifact {
    fn from(row: ArtifactRow) -> Self {
        Self {
            id: row.id,
            run_id: row.run_id,
            step_id: row.step_id,
            name: row.name,
            storage_key: row.storage_key,
            content_type: row.content_type,
            size_bytes: row.size_bytes.max(0) as u64,
            sha256: row.sha256,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

impl ArtifactStore for PostgresStore {
    fn create_artifact(&self, artifact: NewArtifact) -> StoreFuture<'_, Artifact> {
        Box::pin(async move {
            let row = sqlx::query_as!(
                ArtifactRow,
                r#"
                INSERT INTO ironflow.step_artifacts
                    (id, run_id, step_id, name, storage_key, content_type, size_bytes, sha256)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                RETURNING id, run_id, step_id, name, storage_key, content_type, size_bytes, sha256, created_at, updated_at
                "#,
                artifact.id,
                artifact.run_id,
                artifact.step_id,
                &artifact.name,
                &artifact.storage_key,
                &artifact.content_type,
                artifact.size_bytes as i64,
                &artifact.sha256,
            )
            .fetch_one(&self.pool)
            .await
            .map_err(|e| match e.as_database_error().and_then(|db| db.code()) {
                Some(code) if code == UNIQUE_VIOLATION => StoreError::DuplicateArtifact {
                    step_id: artifact.step_id,
                    name: artifact.name.clone(),
                },
                Some(code) if code == FOREIGN_KEY_VIOLATION => {
                    StoreError::StepNotFound(artifact.step_id)
                }
                _ => StoreError::Database(e.to_string()),
            })?;

            Ok(row.into())
        })
    }

    fn get_artifact(&self, step_id: Uuid, name: &str) -> StoreFuture<'_, Option<Artifact>> {
        let name = name.to_string();
        Box::pin(async move {
            let row = sqlx::query_as!(
                ArtifactRow,
                r#"
                SELECT id, run_id, step_id, name, storage_key, content_type, size_bytes, sha256, created_at, updated_at
                FROM ironflow.step_artifacts
                WHERE step_id = $1 AND name = $2
                "#,
                step_id,
                &name,
            )
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(row.map(Artifact::from))
        })
    }

    fn list_artifacts_for_run(&self, run_id: Uuid) -> StoreFuture<'_, Vec<Artifact>> {
        Box::pin(async move {
            let rows = sqlx::query_as!(
                ArtifactRow,
                r#"
                SELECT a.id, a.run_id, a.step_id, a.name, a.storage_key, a.content_type, a.size_bytes, a.sha256, a.created_at, a.updated_at
                FROM ironflow.step_artifacts a
                JOIN ironflow.steps s ON s.id = a.step_id
                WHERE a.run_id = $1
                ORDER BY s.attempt ASC, s.position ASC, a.name ASC
                "#,
                run_id,
            )
            .fetch_all(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(rows.into_iter().map(Artifact::from).collect())
        })
    }

    fn find_artifact_for_input(&self, lookup: ArtifactLookup) -> StoreFuture<'_, Option<Artifact>> {
        Box::pin(async move {
            let row = sqlx::query_as!(
                ArtifactRow,
                r#"
                SELECT a.id, a.run_id, a.step_id, a.name, a.storage_key, a.content_type, a.size_bytes, a.sha256, a.created_at, a.updated_at
                FROM ironflow.step_artifacts a
                JOIN ironflow.steps s ON s.id = a.step_id
                WHERE a.run_id = $1
                  AND s.attempt = $2
                  AND s.name = $3
                  AND s.position < $4
                  AND a.name = $5
                ORDER BY s.position DESC
                LIMIT 1
                "#,
                lookup.run_id,
                lookup.attempt as i32,
                &lookup.step_name,
                lookup.before_position as i32,
                &lookup.name,
            )
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(row.map(Artifact::from))
        })
    }
}
