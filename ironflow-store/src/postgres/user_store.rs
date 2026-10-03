use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

use crate::entities::{NewRefreshToken, NewUser, Page, User};
use crate::error::StoreError;
use crate::store::StoreFuture;
use crate::user_store::UserStore;

use super::PostgresStore;

/// Intermediate row struct matching the `iam.users` columns exactly.
#[derive(FromRow)]
struct UserRow {
    id: Uuid,
    email: String,
    username: String,
    password_hash: String,
    is_admin: bool,
    token_version: i64,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<UserRow> for User {
    fn from(row: UserRow) -> Self {
        Self {
            id: row.id,
            email: row.email,
            username: row.username,
            password_hash: row.password_hash,
            is_admin: row.is_admin,
            token_version: row.token_version,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// Row struct for paginated queries that include `total_count`.
#[derive(FromRow)]
struct UserRowWithTotal {
    id: Uuid,
    email: String,
    username: String,
    password_hash: String,
    is_admin: bool,
    token_version: i64,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    total_count: i64,
}

impl UserStore for PostgresStore {
    fn create_user(&self, req: NewUser) -> StoreFuture<'_, User> {
        Box::pin(async move {
            let id = Uuid::now_v7();
            let now = Utc::now();

            let mut tx = self
                .pool
                .begin()
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            let is_admin = match req.is_admin {
                Some(v) => v,
                None => {
                    let count =
                        sqlx::query_scalar!("SELECT COUNT(*) as \"cnt!: i64\" FROM iam.users")
                            .fetch_one(&mut *tx)
                            .await
                            .map_err(|e| StoreError::Database(e.to_string()))?;
                    count == 0
                }
            };

            let row = sqlx::query_as::<_, UserRow>(
                r#"
                INSERT INTO iam.users (id, email, username, password_hash, is_admin, created_at, updated_at)
                VALUES ($1, $2, $3, $4, $5, $6, $7)
                RETURNING id, email, username, password_hash, is_admin, token_version, created_at, updated_at
                "#,
            )
            .bind(id)
            .bind(&req.email)
            .bind(&req.username)
            .bind(&req.password_hash)
            .bind(is_admin)
            .bind(now)
            .bind(now)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| {
                let msg = e.to_string();
                if msg.contains("users_email_key") || (msg.contains("unique") && msg.contains("email")) {
                    StoreError::DuplicateEmail(req.email.clone())
                } else if msg.contains("users_username_key") || (msg.contains("unique") && msg.contains("username")) {
                    StoreError::DuplicateUsername(req.username.clone())
                } else {
                    StoreError::Database(msg)
                }
            })?;

            tx.commit()
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(row.into())
        })
    }

    fn find_user_by_email(&self, email: &str) -> StoreFuture<'_, Option<User>> {
        let email = email.to_string();
        Box::pin(async move {
            let row = sqlx::query_as::<_, UserRow>(
                "SELECT id, email, username, password_hash, is_admin, token_version, created_at, updated_at FROM iam.users WHERE email = $1",
            )
            .bind(&email)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(row.map(User::from))
        })
    }

    fn find_user_by_username(&self, username: &str) -> StoreFuture<'_, Option<User>> {
        let username = username.to_string();
        Box::pin(async move {
            let row = sqlx::query_as::<_, UserRow>(
                "SELECT id, email, username, password_hash, is_admin, token_version, created_at, updated_at FROM iam.users WHERE username = $1",
            )
            .bind(&username)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(row.map(User::from))
        })
    }

    fn find_user_by_id(&self, id: Uuid) -> StoreFuture<'_, Option<User>> {
        Box::pin(async move {
            let row = sqlx::query_as::<_, UserRow>(
                "SELECT id, email, username, password_hash, is_admin, token_version, created_at, updated_at FROM iam.users WHERE id = $1",
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(row.map(User::from))
        })
    }

    fn count_users(&self) -> StoreFuture<'_, u64> {
        Box::pin(async move {
            let count = sqlx::query_scalar!("SELECT COUNT(*) as \"cnt!: i64\" FROM iam.users")
                .fetch_one(&self.pool)
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;
            Ok(count as u64)
        })
    }

    fn list_users(&self, page: u32, per_page: u32) -> StoreFuture<'_, Page<User>> {
        Box::pin(async move {
            let offset = (page.saturating_sub(1) as i64) * (per_page as i64);
            let rows = sqlx::query_as::<_, UserRowWithTotal>(
                r#"
                SELECT id, email, username, password_hash, is_admin, token_version, created_at, updated_at,
                       COUNT(*) OVER() AS total_count
                FROM iam.users
                ORDER BY created_at DESC
                LIMIT $1 OFFSET $2
                "#,
            )
            .bind(per_page as i64)
            .bind(offset)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            let total = rows.first().map_or(0u64, |r| r.total_count as u64);

            let items = rows
                .into_iter()
                .map(|r| User {
                    id: r.id,
                    email: r.email,
                    username: r.username,
                    password_hash: r.password_hash,
                    is_admin: r.is_admin,
                    token_version: r.token_version,
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

    fn delete_user(&self, id: Uuid) -> StoreFuture<'_, ()> {
        Box::pin(async move {
            let result = sqlx::query!("DELETE FROM iam.users WHERE id = $1", id,)
                .execute(&self.pool)
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            if result.rows_affected() == 0 {
                return Err(StoreError::UserNotFound(id));
            }
            Ok(())
        })
    }

    fn update_user_role(&self, id: Uuid, is_admin: bool) -> StoreFuture<'_, User> {
        Box::pin(async move {
            let mut tx = self
                .pool
                .begin()
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            let row = sqlx::query_as::<_, UserRow>(
                r#"
                UPDATE iam.users SET is_admin = $1, token_version = token_version + 1, updated_at = $2
                WHERE id = $3
                RETURNING id, email, username, password_hash, is_admin, token_version, created_at, updated_at
                "#,
            )
            .bind(is_admin)
            .bind(Utc::now())
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?
            .ok_or(StoreError::UserNotFound(id))?;

            sqlx::query("DELETE FROM iam.refresh_tokens WHERE user_id = $1")
                .bind(id)
                .execute(&mut *tx)
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            tx.commit()
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(row.into())
        })
    }

    fn update_user_password(&self, id: Uuid, password_hash: String) -> StoreFuture<'_, ()> {
        Box::pin(async move {
            let mut tx = self
                .pool
                .begin()
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            let result = sqlx::query(
                "UPDATE iam.users SET password_hash = $1, token_version = token_version + 1, updated_at = $2 WHERE id = $3",
            )
            .bind(&password_hash)
            .bind(Utc::now())
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            if result.rows_affected() == 0 {
                return Err(StoreError::UserNotFound(id));
            }

            sqlx::query("DELETE FROM iam.refresh_tokens WHERE user_id = $1")
                .bind(id)
                .execute(&mut *tx)
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            tx.commit()
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(())
        })
    }

    fn list_user_groups(&self, user_id: Uuid) -> StoreFuture<'_, Vec<String>> {
        Box::pin(async move {
            sqlx::query_scalar::<_, String>(
                "SELECT group_name FROM iam.user_groups WHERE user_id = $1 ORDER BY group_name",
            )
            .bind(user_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))
        })
    }

    fn set_user_groups(&self, user_id: Uuid, groups: Vec<String>) -> StoreFuture<'_, Vec<String>> {
        Box::pin(async move {
            let groups: Vec<String> = groups
                .into_iter()
                .collect::<BTreeSet<String>>()
                .into_iter()
                .collect();

            let mut tx = self
                .pool
                .begin()
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            let exists = sqlx::query_scalar::<_, i32>("SELECT 1 FROM iam.users WHERE id = $1")
                .bind(user_id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;
            if exists.is_none() {
                return Err(StoreError::UserNotFound(user_id));
            }

            sqlx::query("DELETE FROM iam.user_groups WHERE user_id = $1")
                .bind(user_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            sqlx::query(
                "INSERT INTO iam.user_groups (user_id, group_name) \
                 SELECT $1, UNNEST($2::text[]) ON CONFLICT DO NOTHING",
            )
            .bind(user_id)
            .bind(&groups)
            .execute(&mut *tx)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            tx.commit()
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(groups)
        })
    }

    fn revoke_user_sessions(&self, id: Uuid) -> StoreFuture<'_, i64> {
        Box::pin(async move {
            let mut tx = self
                .pool
                .begin()
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            let version = sqlx::query_scalar::<_, i64>(
                "UPDATE iam.users SET token_version = token_version + 1, updated_at = NOW() WHERE id = $1 RETURNING token_version",
            )
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?
            .ok_or(StoreError::UserNotFound(id))?;

            sqlx::query("DELETE FROM iam.refresh_tokens WHERE user_id = $1")
                .bind(id)
                .execute(&mut *tx)
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            tx.commit()
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(version)
        })
    }

    fn store_refresh_token(&self, token: NewRefreshToken) -> StoreFuture<'_, ()> {
        Box::pin(async move {
            let mut tx = self
                .pool
                .begin()
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            sqlx::query("DELETE FROM iam.refresh_tokens WHERE user_id = $1 AND expires_at <= NOW()")
            .bind(token.user_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            sqlx::query(
                "INSERT INTO iam.refresh_tokens (token_hash, user_id, expires_at) VALUES ($1, $2, $3)",
            )
            .bind(&token.token_hash)
            .bind(token.user_id)
            .bind(token.expires_at)
            .execute(&mut *tx)
            .await
            .map_err(|e| {
                let msg = e.to_string();
                if msg.contains("refresh_tokens_user_id_fkey") || msg.contains("foreign key") {
                    StoreError::UserNotFound(token.user_id)
                } else {
                    StoreError::Database(msg)
                }
            })?;

            tx.commit()
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(())
        })
    }

    fn consume_refresh_token(&self, token_hash: &str) -> StoreFuture<'_, Option<Uuid>> {
        let token_hash = token_hash.to_string();
        Box::pin(async move {
            // DELETE ... RETURNING is atomic: of two concurrent refreshes with
            // the same token, only one gets the row back.
            let row = sqlx::query_as::<_, (Uuid, DateTime<Utc>)>(
                "DELETE FROM iam.refresh_tokens WHERE token_hash = $1 RETURNING user_id, expires_at",
            )
            .bind(&token_hash)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            let live = row.filter(|(_, expires_at)| *expires_at > Utc::now());
            Ok(live.map(|(user_id, _)| user_id))
        })
    }
}
