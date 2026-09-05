//! [`SecretStore`] implementation for PostgreSQL.

use chrono::{DateTime, Utc};
#[cfg(feature = "secret-store")]
use tracing::error;
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
        let rows = sqlx::query_scalar!(
            "SELECT DISTINCT key_version FROM ironflow.secrets ORDER BY key_version ASC"
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StoreError::Database(e.to_string()))?;

        Ok(rows)
    }
}

/// Row struct for secret queries that return metadata + crypto columns.
#[cfg(feature = "secret-store")]
struct SecretCryptoRow {
    id: Uuid,
    key: String,
    encrypted_value: Vec<u8>,
    nonce: Vec<u8>,
    key_version: i32,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

/// Row struct for `set_secret` RETURNING clause (no encrypted columns).
#[cfg(feature = "secret-store")]
struct SecretUpsertRow {
    id: Uuid,
    key: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

/// Row struct for rotation batch (no timestamps).
#[cfg(feature = "secret-store")]
struct SecretRotationRow {
    id: Uuid,
    key: String,
    encrypted_value: Vec<u8>,
    nonce: Vec<u8>,
    key_version: i32,
}

/// Row struct for paginated metadata listing.
struct SecretMetadataRow {
    id: Uuid,
    key: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    total_count: i64,
}

impl SecretStore for PostgresStore {
    fn get_secret(&self, key: &str) -> StoreFuture<'_, Option<Secret>> {
        let key = key.to_string();
        Box::pin(async move {
            #[cfg(feature = "secret-store")]
            {
                let ring = self.require_key_ring()?;

                let row = sqlx::query_as!(
                    SecretCryptoRow,
                    "SELECT id, key, encrypted_value, nonce, key_version, created_at, updated_at FROM ironflow.secrets WHERE key = $1",
                    &key,
                )
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

                let Some(row) = row else {
                    return Ok(None);
                };

                let master_key = ring.key_for(row.key_version).ok_or_else(|| {
                    StoreError::Crypto(format!(
                        "secret {:?} uses key version {} which is not configured",
                        key, row.key_version
                    ))
                })?;

                let plaintext = decrypt(master_key, &row.encrypted_value, &row.nonce)
                    .map_err(|e| StoreError::Crypto(e.to_string()))?;

                let value = String::from_utf8(plaintext)
                    .map_err(|e| StoreError::Crypto(format!("invalid UTF-8: {e}")))?;

                Ok(Some(Secret {
                    id: row.id,
                    key: row.key,
                    value,
                    created_at: row.created_at,
                    updated_at: row.updated_at,
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

                let row = sqlx::query_as!(
                    SecretUpsertRow,
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
                    id,
                    &key,
                    &encrypted_value,
                    &nonce,
                    ring.active_version(),
                    now,
                    now,
                )
                .fetch_one(&self.pool)
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

                Ok(Secret {
                    id: row.id,
                    key: row.key,
                    value,
                    created_at: row.created_at,
                    updated_at: row.updated_at,
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
            let result = sqlx::query!("DELETE FROM ironflow.secrets WHERE key = $1", &key,)
                .execute(&self.pool)
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(result.rows_affected() > 0)
        })
    }

    fn list_secret_keys(&self, prefix: &str) -> StoreFuture<'_, Vec<String>> {
        let pattern = format!("{}%", escape_like(prefix));
        Box::pin(async move {
            let rows = sqlx::query_scalar!(
                "SELECT key FROM ironflow.secrets WHERE key LIKE $1 ESCAPE '\\' ORDER BY key",
                &pattern,
            )
            .fetch_all(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(rows)
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

            let rows = sqlx::query_as!(
                SecretMetadataRow,
                r#"
                SELECT id, key, created_at, updated_at, COUNT(*) OVER() as "total_count!: i64"
                FROM ironflow.secrets
                WHERE key LIKE $1 ESCAPE '\'
                ORDER BY key ASC
                LIMIT $2 OFFSET $3
                "#,
                &pattern,
                per_page as i64,
                offset,
            )
            .fetch_all(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            let total = rows.first().map_or(0u64, |r| r.total_count as u64);

            let items = rows
                .into_iter()
                .map(|r| SecretMetadata {
                    id: r.id,
                    key: r.key,
                    created_at: r.created_at,
                    updated_at: r.updated_at,
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

                let rows = sqlx::query_as!(
                    SecretRotationRow,
                    r#"
                    SELECT id, key, encrypted_value, nonce, key_version
                    FROM ironflow.secrets
                    WHERE key_version <> $1
                      AND ($2::uuid IS NULL OR id > $2)
                    ORDER BY id ASC
                    LIMIT $3
                    "#,
                    request.to_version,
                    request.after_id,
                    batch_size,
                )
                .fetch_all(&self.pool)
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

                let mut rotated = 0u64;
                let mut failed = 0u64;
                let mut last_id: Option<Uuid> = None;

                for row in &rows {
                    last_id = Some(row.id);

                    let plaintext = match ring
                        .key_for(row.key_version)
                        .ok_or_else(|| format!("key version {} is not configured", row.key_version))
                        .and_then(|source| {
                            decrypt(source, &row.encrypted_value, &row.nonce)
                                .map_err(|e| e.to_string())
                        }) {
                        Ok(plaintext) => plaintext,
                        Err(reason) => {
                            failed += 1;
                            error!(
                                secret_key = %row.key,
                                key_version = row.key_version,
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
                                secret_key = %row.key,
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
                    let result = sqlx::query!(
                        r#"
                        UPDATE ironflow.secrets
                        SET encrypted_value = $1, nonce = $2, key_version = $3
                        WHERE id = $4 AND key_version = $5
                        "#,
                        &new_value,
                        &new_nonce,
                        request.to_version,
                        row.id,
                        row.key_version,
                    )
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
                let remaining = sqlx::query_scalar!(
                    r#"
                    SELECT COUNT(*) AS "remaining!: i64"
                    FROM ironflow.secrets
                    WHERE key_version <> $1
                      AND ($2::uuid IS NULL OR id > $2)
                    "#,
                    request.to_version,
                    cursor,
                )
                .fetch_one(&self.pool)
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

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
