//! [`SecretStore`] implementation for PostgreSQL.

#[cfg(feature = "secret-store")]
use chrono::Utc;
use sqlx::Row;
#[cfg(feature = "secret-store")]
use tracing::error;
#[cfg(feature = "secret-store")]
use uuid::Uuid;

#[cfg(feature = "secret-store")]
use crate::crypto::{decrypt, encrypt, join_versions};
use crate::entities::{
    KeyVersionStatus, Page, RotationBatch, RotationRequest, Secret, SecretMetadata,
};
use crate::error::StoreError;
use crate::secret_store::SecretStore;
use crate::store::StoreFuture;

use super::PostgresStore;

/// Escape LIKE wildcards (`%`, `_`) so they are matched literally.
fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

#[cfg(feature = "secret-store")]
impl PostgresStore {
    /// The configured key ring, or a [`StoreError::Crypto`] naming what is missing.
    fn require_key_ring(&self) -> Result<&crate::crypto::KeyRing, StoreError> {
        self.key_ring
            .as_deref()
            .ok_or_else(|| StoreError::Crypto("no master key configured".to_string()))
    }

    /// Distinct key versions used by stored secrets, ascending.
    async fn used_key_versions(&self) -> Result<Vec<i32>, StoreError> {
        let rows = sqlx::query(
            "SELECT DISTINCT key_version FROM ironflow.secrets ORDER BY key_version ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StoreError::Database(e.to_string()))?;

        Ok(rows.iter().map(|r| r.get("key_version")).collect())
    }
}

impl SecretStore for PostgresStore {
    fn get_secret(&self, key: &str) -> StoreFuture<'_, Option<Secret>> {
        let key = key.to_string();
        Box::pin(async move {
            #[cfg(feature = "secret-store")]
            {
                let ring = self.require_key_ring()?;

                let row = sqlx::query(
                    "SELECT id, key, encrypted_value, nonce, key_version, created_at, updated_at FROM ironflow.secrets WHERE key = $1",
                )
                .bind(&key)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

                let Some(row) = row else {
                    return Ok(None);
                };

                let encrypted_value: Vec<u8> = row.get("encrypted_value");
                let nonce: Vec<u8> = row.get("nonce");
                let key_version: i32 = row.get("key_version");

                let master_key = ring.key_for(key_version).ok_or_else(|| {
                    StoreError::Crypto(format!(
                        "secret {key:?} uses key version {key_version} which is not configured"
                    ))
                })?;

                let plaintext = decrypt(master_key, &encrypted_value, &nonce)
                    .map_err(|e| StoreError::Crypto(e.to_string()))?;

                let value = String::from_utf8(plaintext)
                    .map_err(|e| StoreError::Crypto(format!("invalid UTF-8: {e}")))?;

                Ok(Some(Secret {
                    id: row.get("id"),
                    key: row.get("key"),
                    value,
                    created_at: row.get("created_at"),
                    updated_at: row.get("updated_at"),
                }))
            }
            #[cfg(not(feature = "secret-store"))]
            {
                let _ = key;
                Err(StoreError::Crypto(
                    "secret-store feature not enabled".to_string(),
                ))
            }
        })
    }

    fn set_secret(&self, key: &str, value: &str) -> StoreFuture<'_, Secret> {
        let key = key.to_string();
        let value = value.to_string();
        Box::pin(async move {
            #[cfg(feature = "secret-store")]
            {
                let ring = self.require_key_ring()?;

                let (encrypted_value, nonce) = encrypt(ring.active_key(), value.as_bytes())
                    .map_err(|e| StoreError::Crypto(e.to_string()))?;

                let id = Uuid::now_v7();
                let now = Utc::now();

                let row = sqlx::query(
                    r#"
                    INSERT INTO ironflow.secrets (id, key, encrypted_value, nonce, key_version, created_at, updated_at)
                    VALUES ($1, $2, $3, $4, $5, $6, $7)
                    ON CONFLICT (key) DO UPDATE
                        SET encrypted_value = EXCLUDED.encrypted_value,
                            nonce = EXCLUDED.nonce,
                            key_version = EXCLUDED.key_version,
                            updated_at = EXCLUDED.updated_at
                    RETURNING id, key, created_at, updated_at
                    "#,
                )
                .bind(id)
                .bind(&key)
                .bind(&encrypted_value)
                .bind(&nonce)
                .bind(ring.active_version())
                .bind(now)
                .bind(now)
                .fetch_one(&self.pool)
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

                Ok(Secret {
                    id: row.get("id"),
                    key: row.get("key"),
                    value,
                    created_at: row.get("created_at"),
                    updated_at: row.get("updated_at"),
                })
            }
            #[cfg(not(feature = "secret-store"))]
            {
                let _ = (key, value);
                Err(StoreError::Crypto(
                    "secret-store feature not enabled".to_string(),
                ))
            }
        })
    }

    fn delete_secret(&self, key: &str) -> StoreFuture<'_, bool> {
        let key = key.to_string();
        Box::pin(async move {
            let result = sqlx::query("DELETE FROM ironflow.secrets WHERE key = $1")
                .bind(&key)
                .execute(&self.pool)
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(result.rows_affected() > 0)
        })
    }

    fn list_secret_keys(&self, prefix: &str) -> StoreFuture<'_, Vec<String>> {
        let pattern = format!("{}%", escape_like(prefix));
        Box::pin(async move {
            let rows = sqlx::query(
                "SELECT key FROM ironflow.secrets WHERE key LIKE $1 ESCAPE '\\' ORDER BY key",
            )
            .bind(&pattern)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(rows.iter().map(|r| r.get("key")).collect())
        })
    }

    fn list_secrets(
        &self,
        prefix: &str,
        page: u32,
        per_page: u32,
    ) -> StoreFuture<'_, Page<SecretMetadata>> {
        let pattern = format!("{}%", escape_like(prefix));
        Box::pin(async move {
            let page = page.max(1);
            let per_page = per_page.clamp(1, 100);
            let offset = ((page - 1) * per_page) as i64;

            let rows = sqlx::query(
                r#"
                SELECT id, key, created_at, updated_at, COUNT(*) OVER() as total_count
                FROM ironflow.secrets
                WHERE key LIKE $1 ESCAPE '\'
                ORDER BY key ASC
                LIMIT $2 OFFSET $3
                "#,
            )
            .bind(&pattern)
            .bind(per_page as i64)
            .bind(offset)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            let total = if rows.is_empty() {
                0u64
            } else {
                rows[0].get::<i64, _>("total_count") as u64
            };

            let items = rows
                .into_iter()
                .map(|r| SecretMetadata {
                    id: r.get("id"),
                    key: r.get("key"),
                    created_at: r.get("created_at"),
                    updated_at: r.get("updated_at"),
                })
                .collect();

            Ok(Page {
                items,
                total,
                page,
                per_page,
            })
        })
    }

    fn secret_key_status(&self) -> StoreFuture<'_, KeyVersionStatus> {
        Box::pin(async move {
            #[cfg(feature = "secret-store")]
            {
                let ring = self.require_key_ring()?;
                let in_use = self.used_key_versions().await?;

                Ok(KeyVersionStatus {
                    active: ring.active_version(),
                    configured: ring.versions(),
                    missing: ring.missing_versions(&in_use),
                    retirable: ring.retirable_versions(&in_use),
                    in_use,
                })
            }
            #[cfg(not(feature = "secret-store"))]
            {
                Err(StoreError::Crypto(
                    "secret-store feature not enabled".to_string(),
                ))
            }
        })
    }

    fn rotate_secrets(&self, request: RotationRequest) -> StoreFuture<'_, RotationBatch> {
        Box::pin(async move {
            #[cfg(feature = "secret-store")]
            {
                let ring = self.require_key_ring()?;
                let target = ring.key_for(request.to_version).ok_or_else(|| {
                    StoreError::Crypto(format!(
                        "target key version {} is not configured (available: {})",
                        request.to_version,
                        join_versions(&ring.versions())
                    ))
                })?;

                let batch_size = request.effective_batch_size() as i64;

                let rows = sqlx::query(
                    r#"
                    SELECT id, key, encrypted_value, nonce, key_version
                    FROM ironflow.secrets
                    WHERE key_version <> $1
                      AND ($2::uuid IS NULL OR id > $2)
                    ORDER BY id ASC
                    LIMIT $3
                    "#,
                )
                .bind(request.to_version)
                .bind(request.after_id)
                .bind(batch_size)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

                let mut rotated = 0u64;
                let mut failed = 0u64;
                let mut last_id: Option<Uuid> = None;

                for row in &rows {
                    let id: Uuid = row.get("id");
                    let secret_key: String = row.get("key");
                    let source_version: i32 = row.get("key_version");
                    let encrypted_value: Vec<u8> = row.get("encrypted_value");
                    let nonce: Vec<u8> = row.get("nonce");

                    last_id = Some(id);

                    let plaintext = match ring
                        .key_for(source_version)
                        .ok_or_else(|| format!("key version {source_version} is not configured"))
                        .and_then(|source| {
                            decrypt(source, &encrypted_value, &nonce).map_err(|e| e.to_string())
                        }) {
                        Ok(plaintext) => plaintext,
                        Err(reason) => {
                            failed += 1;
                            error!(
                                secret_key = %secret_key,
                                key_version = source_version,
                                reason = %reason,
                                "secret rotation failed"
                            );
                            continue;
                        }
                    };

                    let (new_value, new_nonce) = match encrypt(target, &plaintext) {
                        Ok(pair) => pair,
                        Err(e) => {
                            failed += 1;
                            error!(
                                secret_key = %secret_key,
                                reason = %e,
                                "secret rotation failed"
                            );
                            continue;
                        }
                    };

                    // The key_version guard makes this a compare-and-swap: a
                    // concurrent set_secret on the same key wins, and its row
                    // is left alone rather than overwritten with stale data.
                    // updated_at stays untouched: re-encryption is not a
                    // change to the secret itself.
                    let result = sqlx::query(
                        r#"
                        UPDATE ironflow.secrets
                        SET encrypted_value = $1, nonce = $2, key_version = $3
                        WHERE id = $4 AND key_version = $5
                        "#,
                    )
                    .bind(&new_value)
                    .bind(&new_nonce)
                    .bind(request.to_version)
                    .bind(id)
                    .bind(source_version)
                    .execute(&self.pool)
                    .await
                    .map_err(|e| StoreError::Database(e.to_string()))?;

                    if result.rows_affected() > 0 {
                        rotated += 1;
                    }
                }

                // Count from where this batch stopped. An empty batch leaves
                // last_id unset, so fall back to the incoming cursor rather
                // than counting the whole stock again.
                let cursor = last_id.or(request.after_id);
                let remaining: i64 = sqlx::query(
                    r#"
                    SELECT COUNT(*) AS remaining
                    FROM ironflow.secrets
                    WHERE key_version <> $1
                      AND ($2::uuid IS NULL OR id > $2)
                    "#,
                )
                .bind(request.to_version)
                .bind(cursor)
                .fetch_one(&self.pool)
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?
                .get("remaining");

                Ok(RotationBatch {
                    to_version: request.to_version,
                    rotated,
                    failed,
                    remaining: remaining.max(0) as u64,
                    last_id,
                })
            }
            #[cfg(not(feature = "secret-store"))]
            {
                let _ = request;
                Err(StoreError::Crypto(
                    "secret-store feature not enabled".to_string(),
                ))
            }
        })
    }
}
