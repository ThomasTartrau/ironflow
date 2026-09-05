use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::entities::{NewUser, Page, User};
use crate::error::StoreError;
use crate::store::StoreFuture;
use crate::user_store::UserStore;

use super::PostgresStore;

/// Intermediate row struct matching the `iam.users` columns exactly.
struct UserRow {
    id: Uuid,
    email: String,
    username: String,
    password_hash: String,
    is_admin: bool,
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
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// Row struct for paginated queries that include `total_count`.
struct UserRowWithTotal {
    id: Uuid,
    email: String,
    username: String,
    password_hash: String,
    is_admin: bool,
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

            let row = sqlx::query_as!(
                UserRow,
                r#"
                INSERT INTO iam.users (id, email, username, password_hash, is_admin, created_at, updated_at)
                VALUES ($1, $2, $3, $4, $5, $6, $7)
                RETURNING id, email, username, password_hash, is_admin, created_at, updated_at
                "#,
                id,
                &req.email,
                &req.username,
                &req.password_hash,
                is_admin,
                now,
                now,
            )
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
            let row = sqlx::query_as!(
                UserRow,
                "SELECT id, email, username, password_hash, is_admin, created_at, updated_at FROM iam.users WHERE email = $1",
                &email,
            )
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(row.map(User::from))
        })
    }

    fn find_user_by_id(&self, id: Uuid) -> StoreFuture<'_, Option<User>> {
        Box::pin(async move {
            let row = sqlx::query_as!(
                UserRow,
                "SELECT id, email, username, password_hash, is_admin, created_at, updated_at FROM iam.users WHERE id = $1",
                id,
            )
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
            let rows = sqlx::query_as!(
                UserRowWithTotal,
                r#"
                SELECT id, email, username, password_hash, is_admin, created_at, updated_at,
                       COUNT(*) OVER() as "total_count!: i64"
                FROM iam.users
                ORDER BY created_at DESC
                LIMIT $1 OFFSET $2
                "#,
                per_page as i64,
                offset,
            )
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
            let row = sqlx::query_as!(
                UserRow,
                r#"
                UPDATE iam.users SET is_admin = $1, updated_at = $2
                WHERE id = $3
                RETURNING id, email, username, password_hash, is_admin, created_at, updated_at
                "#,
                is_admin,
                Utc::now(),
                id,
            )
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?
            .ok_or(StoreError::UserNotFound(id))?;

            Ok(row.into())
        })
    }
}
