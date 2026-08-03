//! [`ArtifactStore`] implementation for [`PostgresStore`].

use sqlx::Row;
use sqlx::postgres::PgRow;
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

/// Columns selected by every artifact query, in [`row_to_artifact`] order.
const ARTIFACT_COLUMNS: &str = "id, run_id, step_id, name, storage_key, content_type, size_bytes, sha256, created_at, updated_at";

fn row_to_artifact(row: PgRow) -> Artifact {
    let size_bytes: i64 = row.get("size_bytes");

    Artifact {
        id: row.get("id"),
        run_id: row.get("run_id"),
        step_id: row.get("step_id"),
        name: row.get("name"),
        storage_key: row.get("storage_key"),
        content_type: row.get("content_type"),
        // The column carries a `size_bytes >= 0` check constraint.
        size_bytes: size_bytes.max(0) as u64,
        sha256: row.get("sha256"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

impl ArtifactStore for PostgresStore {
    fn create_artifact(&self, artifact: NewArtifact) -> StoreFuture<'_, Artifact> {
        Box::pin(async move {
            let sql = format!(
                r#"
                INSERT INTO ironflow.step_artifacts
                    (id, run_id, step_id, name, storage_key, content_type, size_bytes, sha256)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                RETURNING {ARTIFACT_COLUMNS}
                "#
            );

            let row = sqlx::query(&sql)
                .bind(artifact.id)
                .bind(artifact.run_id)
                .bind(artifact.step_id)
                .bind(&artifact.name)
                .bind(&artifact.storage_key)
                .bind(&artifact.content_type)
                .bind(artifact.size_bytes as i64)
                .bind(&artifact.sha256)
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

            Ok(row_to_artifact(row))
        })
    }

    fn get_artifact(&self, step_id: Uuid, name: &str) -> StoreFuture<'_, Option<Artifact>> {
        let name = name.to_string();
        Box::pin(async move {
            let sql = format!(
                r#"
                SELECT {ARTIFACT_COLUMNS}
                FROM ironflow.step_artifacts
                WHERE step_id = $1 AND name = $2
                "#
            );

            let row = sqlx::query(&sql)
                .bind(step_id)
                .bind(&name)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(row.map(row_to_artifact))
        })
    }

    fn list_artifacts_for_run(&self, run_id: Uuid) -> StoreFuture<'_, Vec<Artifact>> {
        Box::pin(async move {
            // Joined to steps so the ordering follows the execution timeline,
            // which is how the dashboard renders them.
            let sql = format!(
                r#"
                SELECT {}
                FROM ironflow.step_artifacts a
                JOIN ironflow.steps s ON s.id = a.step_id
                WHERE a.run_id = $1
                ORDER BY s.attempt ASC, s.position ASC, a.name ASC
                "#,
                ARTIFACT_COLUMNS
                    .split(", ")
                    .map(|column| format!("a.{column}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            );

            let rows = sqlx::query(&sql)
                .bind(run_id)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(rows.into_iter().map(row_to_artifact).collect())
        })
    }

    fn find_artifact_for_input(&self, lookup: ArtifactLookup) -> StoreFuture<'_, Option<Artifact>> {
        Box::pin(async move {
            let sql = format!(
                r#"
                SELECT {}
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
                ARTIFACT_COLUMNS
                    .split(", ")
                    .map(|column| format!("a.{column}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            );

            let row = sqlx::query(&sql)
                .bind(lookup.run_id)
                .bind(lookup.attempt as i32)
                .bind(&lookup.step_name)
                .bind(lookup.before_position as i32)
                .bind(&lookup.name)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(row.map(row_to_artifact))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_columns_are_prefixable_for_joins() {
        let prefixed: Vec<String> = ARTIFACT_COLUMNS
            .split(", ")
            .map(|column| format!("a.{column}"))
            .collect();

        assert_eq!(prefixed.len(), 10);
        assert!(prefixed.iter().all(|column| column.starts_with("a.")));
        assert!(prefixed.contains(&"a.storage_key".to_string()));
    }
}
