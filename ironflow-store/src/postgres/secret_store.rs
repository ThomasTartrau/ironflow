//! [`SecretStore`] implementation for PostgreSQL.

#[cfg(feature = "secret-store")]
use chrono::Utc;
use sqlx::Row;
#[cfg(feature = "secret-store")]
use uuid::Uuid;

#[cfg(feature = "secret-store")]
use crate::crypto::{decrypt, encrypt};
use crate::entities::{Page, Secret, SecretMetadata};
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

impl SecretStore for PostgresStore {
    fn get_secret(&self, key: &str) -> StoreFuture<'_, Option<Secret>> {
        let key = key.to_string();
        Box::pin(async move {
            #[cfg(feature = "secret-store")]
            {
                let master_key = self
                    .master_key
                    .as_ref()
                    .ok_or_else(|| StoreError::Crypto("no master key configured".to_string()))?;

                let row = sqlx::query(
                    "SELECT id, key, encrypted_value, nonce, created_at, updated_at FROM ironflow.secrets WHERE key = $1",
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
                let master_key = self
                    .master_key
                    .as_ref()
                    .ok_or_else(|| StoreError::Crypto("no master key configured".to_string()))?;

                let (encrypted_value, nonce) = encrypt(master_key, value.as_bytes())
                    .map_err(|e| StoreError::Crypto(e.to_string()))?;

                let id = Uuid::now_v7();
                let now = Utc::now();

                let row = sqlx::query(
                    r#"
                    INSERT INTO ironflow.secrets (id, key, encrypted_value, nonce, created_at, updated_at)
                    VALUES ($1, $2, $3, $4, $5, $6)
                    ON CONFLICT (key) DO UPDATE
                        SET encrypted_value = EXCLUDED.encrypted_value,
                            nonce = EXCLUDED.nonce,
                            updated_at = EXCLUDED.updated_at
                    RETURNING id, key, created_at, updated_at
                    "#,
                )
                .bind(id)
                .bind(&key)
                .bind(&encrypted_value)
                .bind(&nonce)
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
}
