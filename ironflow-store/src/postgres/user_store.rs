use chrono::Utc;
use sqlx::Row;
use uuid::Uuid;

use crate::entities::{NewUser, Page, User};
use crate::error::StoreError;
use crate::store::StoreFuture;
use crate::user_store::UserStore;

use super::PostgresStore;

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
                    let count_row = sqlx::query("SELECT COUNT(*) as cnt FROM iam.users")
                        .fetch_one(&mut *tx)
                        .await
                        .map_err(|e| StoreError::Database(e.to_string()))?;
                    let count: i64 = count_row.get("cnt");
                    count == 0
                }
            };

            let row = sqlx::query(
                r#"
                INSERT INTO iam.users (id, email, username, password_hash, is_admin, created_at, updated_at)
                VALUES ($1, $2, $3, $4, $5, $6, $7)
                RETURNING id, email, username, password_hash, is_admin, created_at, updated_at
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

            Ok(User {
                id: row.get("id"),
                email: row.get("email"),
                username: row.get("username"),
                password_hash: row.get("password_hash"),
                is_admin: row.get("is_admin"),
                created_at: row.get("created_at"),
                updated_at: row.get("updated_at"),
            })
        })
    }

    fn find_user_by_email(&self, email: &str) -> StoreFuture<'_, Option<User>> {
        let email = email.to_string();
        Box::pin(async move {
            let row = sqlx::query(
                "SELECT id, email, username, password_hash, is_admin, created_at, updated_at FROM iam.users WHERE email = $1",
            )
            .bind(&email)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(row.map(|r| User {
                id: r.get("id"),
                email: r.get("email"),
                username: r.get("username"),
                password_hash: r.get("password_hash"),
                is_admin: r.get("is_admin"),
                created_at: r.get("created_at"),
                updated_at: r.get("updated_at"),
            }))
        })
    }

    fn find_user_by_id(&self, id: Uuid) -> StoreFuture<'_, Option<User>> {
        Box::pin(async move {
            let row = sqlx::query(
                "SELECT id, email, username, password_hash, is_admin, created_at, updated_at FROM iam.users WHERE id = $1",
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(row.map(|r| User {
                id: r.get("id"),
                email: r.get("email"),
                username: r.get("username"),
                password_hash: r.get("password_hash"),
                is_admin: r.get("is_admin"),
                created_at: r.get("created_at"),
                updated_at: r.get("updated_at"),
            }))
        })
    }

    fn count_users(&self) -> StoreFuture<'_, u64> {
        Box::pin(async move {
            let row = sqlx::query("SELECT COUNT(*) as cnt FROM iam.users")
                .fetch_one(&self.pool)
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;
            let count: i64 = row.get("cnt");
            Ok(count as u64)
        })
    }

    fn list_users(&self, page: u32, per_page: u32) -> StoreFuture<'_, Page<User>> {
        Box::pin(async move {
            let offset = (page.saturating_sub(1) as i64) * (per_page as i64);
            let rows = sqlx::query(
                r#"
                SELECT id, email, username, password_hash, is_admin, created_at, updated_at,
                       COUNT(*) OVER() as total_count
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

            let total = if rows.is_empty() {
                0u64
            } else {
                rows[0].get::<i64, _>("total_count") as u64
            };

            let items = rows
                .into_iter()
                .map(|r| User {
                    id: r.get("id"),
                    email: r.get("email"),
                    username: r.get("username"),
                    password_hash: r.get("password_hash"),
                    is_admin: r.get("is_admin"),
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

    fn delete_user(&self, id: Uuid) -> StoreFuture<'_, ()> {
        Box::pin(async move {
            let result = sqlx::query("DELETE FROM iam.users WHERE id = $1")
                .bind(id)
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
            let row = sqlx::query(
                r#"
                UPDATE iam.users SET is_admin = $1, updated_at = $2
                WHERE id = $3
                RETURNING id, email, username, password_hash, is_admin, created_at, updated_at
                "#,
            )
            .bind(is_admin)
            .bind(Utc::now())
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?
            .ok_or(StoreError::UserNotFound(id))?;

            Ok(User {
                id: row.get("id"),
                email: row.get("email"),
                username: row.get("username"),
                password_hash: row.get("password_hash"),
                is_admin: row.get("is_admin"),
                created_at: row.get("created_at"),
                updated_at: row.get("updated_at"),
            })
        })
    }
}
