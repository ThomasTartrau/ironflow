use chrono::Utc;
use sqlx::Row;
use uuid::Uuid;

use crate::api_key_store::ApiKeyStore;
use crate::entities::{ApiKey, ApiKeyScope, ApiKeyUpdate, NewApiKey};
use crate::error::StoreError;
use crate::store::StoreFuture;

use super::PostgresStore;

fn parse_scopes(raw: Vec<String>) -> Result<Vec<ApiKeyScope>, StoreError> {
    raw.into_iter()
        .map(|s| {
            s.parse::<ApiKeyScope>()
                .map_err(|e| StoreError::Database(e.to_string()))
        })
        .collect()
}

fn scopes_to_strings(scopes: &[ApiKeyScope]) -> Vec<String> {
    scopes.iter().map(|s| s.to_string()).collect()
}

impl ApiKeyStore for PostgresStore {
    fn create_api_key(&self, req: NewApiKey) -> StoreFuture<'_, ApiKey> {
        Box::pin(async move {
            let id = Uuid::now_v7();
            let now = Utc::now();
            let scopes_str = scopes_to_strings(&req.scopes);

            let row = sqlx::query(
                r#"
                INSERT INTO iam.api_keys (id, user_id, name, key_hash, key_prefix, scopes, is_active, expires_at, created_at, updated_at)
                VALUES ($1, $2, $3, $4, $5, $6, TRUE, $7, $8, $9)
                RETURNING id, user_id, name, key_hash, key_prefix, scopes, is_active, expires_at, last_used_at, created_at, updated_at
                "#,
            )
            .bind(id)
            .bind(req.user_id)
            .bind(&req.name)
            .bind(&req.key_hash)
            .bind(&req.key_prefix)
            .bind(&scopes_str)
            .bind(req.expires_at)
            .bind(now)
            .bind(now)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(ApiKey {
                id: row.get("id"),
                user_id: row.get("user_id"),
                name: row.get("name"),
                key_hash: row.get("key_hash"),
                key_prefix: row.get("key_prefix"),
                scopes: parse_scopes(row.get("scopes"))?,
                is_active: row.get("is_active"),
                expires_at: row.get("expires_at"),
                last_used_at: row.get("last_used_at"),
                created_at: row.get("created_at"),
                updated_at: row.get("updated_at"),
            })
        })
    }

    fn find_api_key_by_prefix(&self, prefix: &str) -> StoreFuture<'_, Option<ApiKey>> {
        let prefix = prefix.to_string();
        Box::pin(async move {
            let row = sqlx::query(
                r#"
                SELECT id, user_id, name, key_hash, key_prefix, scopes, is_active, expires_at, last_used_at, created_at, updated_at
                FROM iam.api_keys WHERE key_prefix = $1 AND is_active = TRUE
                "#,
            )
            .bind(&prefix)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            row.map(|r| {
                Ok(ApiKey {
                    id: r.get("id"),
                    user_id: r.get("user_id"),
                    name: r.get("name"),
                    key_hash: r.get("key_hash"),
                    key_prefix: r.get("key_prefix"),
                    scopes: parse_scopes(r.get("scopes"))?,
                    is_active: r.get("is_active"),
                    expires_at: r.get("expires_at"),
                    last_used_at: r.get("last_used_at"),
                    created_at: r.get("created_at"),
                    updated_at: r.get("updated_at"),
                })
            })
            .transpose()
        })
    }

    fn find_api_key_by_id(&self, id: Uuid) -> StoreFuture<'_, Option<ApiKey>> {
        Box::pin(async move {
            let row = sqlx::query(
                r#"
                SELECT id, user_id, name, key_hash, key_prefix, scopes, is_active, expires_at, last_used_at, created_at, updated_at
                FROM iam.api_keys WHERE id = $1
                "#,
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            row.map(|r| {
                Ok(ApiKey {
                    id: r.get("id"),
                    user_id: r.get("user_id"),
                    name: r.get("name"),
                    key_hash: r.get("key_hash"),
                    key_prefix: r.get("key_prefix"),
                    scopes: parse_scopes(r.get("scopes"))?,
                    is_active: r.get("is_active"),
                    expires_at: r.get("expires_at"),
                    last_used_at: r.get("last_used_at"),
                    created_at: r.get("created_at"),
                    updated_at: r.get("updated_at"),
                })
            })
            .transpose()
        })
    }

    fn list_api_keys_by_user(&self, user_id: Uuid) -> StoreFuture<'_, Vec<ApiKey>> {
        Box::pin(async move {
            let rows = sqlx::query(
                r#"
                SELECT id, user_id, name, key_hash, key_prefix, scopes, is_active, expires_at, last_used_at, created_at, updated_at
                FROM iam.api_keys WHERE user_id = $1 ORDER BY created_at DESC
                "#,
            )
            .bind(user_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            rows.iter()
                .map(|r| {
                    Ok(ApiKey {
                        id: r.get("id"),
                        user_id: r.get("user_id"),
                        name: r.get("name"),
                        key_hash: r.get("key_hash"),
                        key_prefix: r.get("key_prefix"),
                        scopes: parse_scopes(r.get("scopes"))?,
                        is_active: r.get("is_active"),
                        expires_at: r.get("expires_at"),
                        last_used_at: r.get("last_used_at"),
                        created_at: r.get("created_at"),
                        updated_at: r.get("updated_at"),
                    })
                })
                .collect()
        })
    }

    fn update_api_key(&self, id: Uuid, update: ApiKeyUpdate) -> StoreFuture<'_, ()> {
        Box::pin(async move {
            let now = Utc::now();
            let scopes_str: Option<Vec<String>> =
                update.scopes.map(|scopes| scopes_to_strings(&scopes));
            let has_expires_at = update.expires_at.is_some();
            let expires_at_value = update.expires_at.flatten();

            sqlx::query(
                r#"
                UPDATE iam.api_keys
                SET name       = COALESCE($2, name),
                    scopes     = COALESCE($3, scopes),
                    is_active  = COALESCE($4, is_active),
                    expires_at = CASE WHEN $5 THEN $6 ELSE expires_at END,
                    updated_at = $7
                WHERE id = $1
                "#,
            )
            .bind(id)
            .bind(update.name)
            .bind(scopes_str.as_deref())
            .bind(update.is_active)
            .bind(has_expires_at)
            .bind(expires_at_value)
            .bind(now)
            .execute(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(())
        })
    }

    fn touch_api_key(&self, id: Uuid) -> StoreFuture<'_, ()> {
        Box::pin(async move {
            sqlx::query("UPDATE iam.api_keys SET last_used_at = NOW() WHERE id = $1")
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(())
        })
    }

    fn delete_api_key(&self, id: Uuid) -> StoreFuture<'_, ()> {
        Box::pin(async move {
            sqlx::query("DELETE FROM iam.api_keys WHERE id = $1")
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(())
        })
    }
}
