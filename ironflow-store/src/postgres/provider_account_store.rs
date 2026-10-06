//! PostgreSQL [`ProviderAccountStore`] implementation.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use sqlx::Error as SqlxError;
use sqlx::{FromRow, query};
use uuid::Uuid;

use crate::entities::{
    AccountWindowStatus, NewProviderAccount, NewProviderAccountObservation, Page, ProviderAccount,
    ProviderAccountCandidate, ProviderAccountUpdate, ProviderAccountUsagePoint,
    ProviderAccountWindow, ProviderKind,
};
use crate::error::StoreError;
use crate::provider_account_store::ProviderAccountStore;
use crate::store::StoreFuture;

use super::PostgresStore;

/// Postgres error code for a unique constraint violation.
const UNIQUE_VIOLATION: &str = "23505";

fn db_err(e: SqlxError) -> StoreError {
    StoreError::Database(e.to_string())
}

/// `provider_accounts` row. Only `max_concurrency` differs from [`ProviderAccount`]:
/// Postgres has no unsigned integer, and sqlx decodes `INTEGER` as `i32` only.
#[derive(FromRow)]
struct AccountRow {
    id: Uuid,
    name: String,
    display_name: String,
    kind: String,
    secret_key: String,
    enabled: bool,
    priority: i32,
    tags: Vec<String>,
    max_concurrency: Option<i32>,
    alert_threshold: f64,
    expires_at: DateTime<Utc>,
    plan: Option<String>,
    auth_failed_at: Option<DateTime<Utc>>,
    created_by: Option<Uuid>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<AccountRow> for ProviderAccount {
    fn from(row: AccountRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            display_name: row.display_name,
            kind: row.kind,
            secret_key: row.secret_key,
            enabled: row.enabled,
            priority: row.priority,
            tags: row.tags,
            // The column is CHECKed `> 0`.
            max_concurrency: row.max_concurrency.map(|v| v as u32),
            alert_threshold: row.alert_threshold,
            expires_at: row.expires_at,
            plan: row.plan,
            auth_failed_at: row.auth_failed_at,
            created_by: row.created_by,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

impl PostgresStore {
    /// Make every run sleeping on capacity for `kind` due now, so the run
    /// waker resumes it on its next tick: a new, re-enabled or renewed account
    /// may have the capacity it waits for.
    async fn wake_capacity_sleepers(&self, kind: &ProviderKind) -> Result<(), StoreError> {
        query(
            r#"
            UPDATE ironflow.runs r
            SET scheduled_at = NOW(), updated_at = NOW()
            FROM lib_fsm.state_machine sm
            JOIN lib_fsm.abstract_state ast ON ast.abstract_state__id = sm.abstract_state__id
            WHERE sm.state_machine__id = r.state_machine__id
              AND ast.name = 'sleeping'
              AND r.capacity_wait_kind = $1
            "#,
        )
        .bind(kind.as_str())
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn fetch_windows(&self, ids: &[Uuid]) -> Result<Vec<ProviderAccountWindow>, StoreError> {
        // `model_scope = ''` is stored for "every model" because it is part of the key.
        sqlx::query_as!(
            ProviderAccountWindow,
            r#"
            SELECT account_id, window_name AS "window", NULLIF(model_scope, '') AS "model_scope?",
                utilization, resets_at, status AS "status: AccountWindowStatus", observed_at
            FROM ironflow.provider_account_windows
            WHERE account_id = ANY($1)
            ORDER BY account_id, window_name, provider_account_windows.model_scope
            "#,
            ids,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)
    }
}

impl ProviderAccountStore for PostgresStore {
    fn create_provider_account(&self, req: NewProviderAccount) -> StoreFuture<'_, ProviderAccount> {
        Box::pin(async move {
            let row = sqlx::query_as!(
                AccountRow,
                r#"
                INSERT INTO ironflow.provider_accounts
                    (id, name, display_name, kind, secret_key, enabled, priority, tags,
                     max_concurrency, alert_threshold, expires_at, plan, created_by)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
                RETURNING id, name, display_name, kind, secret_key, enabled, priority, tags,
                    max_concurrency, alert_threshold, expires_at, plan, auth_failed_at,
                    created_by, created_at, updated_at
                "#,
                req.id,
                &req.name,
                &req.display_name,
                &req.kind,
                &req.secret_key,
                req.enabled,
                req.priority,
                &req.tags,
                req.max_concurrency.map(|v| v as i32),
                req.alert_threshold,
                req.expires_at,
                req.plan.as_deref(),
                req.created_by,
            )
            .fetch_one(&self.pool)
            .await
            .map_err(|e| match e.as_database_error().and_then(|db| db.code()) {
                Some(code) if code == UNIQUE_VIOLATION => {
                    StoreError::DuplicateProviderAccount(req.name.clone())
                }
                _ => db_err(e),
            })?;
            let account = ProviderAccount::from(row);
            if account.enabled {
                self.wake_capacity_sleepers(&ProviderKind::new(account.kind.as_str())).await?;
            }
            Ok(account)
        })
    }

    fn list_provider_accounts_by_ids(
        &self,
        ids: Vec<Uuid>,
    ) -> StoreFuture<'_, Vec<ProviderAccount>> {
        Box::pin(async move {
            if ids.is_empty() {
                return Ok(Vec::new());
            }
            let rows = sqlx::query_as::<_, AccountRow>(
                "SELECT id, name, display_name, kind, secret_key, enabled, priority, tags, \
                    max_concurrency, alert_threshold, expires_at, plan, auth_failed_at, \
                    created_by, created_at, updated_at \
                FROM ironflow.provider_accounts \
                WHERE id = ANY($1)",
            )
            .bind(&ids)
            .fetch_all(&self.pool)
            .await
            .map_err(db_err)?;
            Ok(rows.into_iter().map(ProviderAccount::from).collect())
        })
    }

    fn get_provider_account(&self, id: Uuid) -> StoreFuture<'_, Option<ProviderAccount>> {
        Box::pin(async move {
            let row = sqlx::query_as!(
                AccountRow,
                r#"
                SELECT id, name, display_name, kind, secret_key, enabled, priority, tags,
                    max_concurrency, alert_threshold, expires_at, plan, auth_failed_at,
                    created_by, created_at, updated_at
                FROM ironflow.provider_accounts
                WHERE id = $1
                "#,
                id,
            )
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?;
            Ok(row.map(ProviderAccount::from))
        })
    }

    fn find_provider_account_by_name(
        &self,
        name: &str,
    ) -> StoreFuture<'_, Option<ProviderAccount>> {
        let name = name.to_string();
        Box::pin(async move {
            let row = sqlx::query_as!(
                AccountRow,
                r#"
                SELECT id, name, display_name, kind, secret_key, enabled, priority, tags,
                    max_concurrency, alert_threshold, expires_at, plan, auth_failed_at,
                    created_by, created_at, updated_at
                FROM ironflow.provider_accounts
                WHERE name = $1
                "#,
                &name,
            )
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?;
            Ok(row.map(ProviderAccount::from))
        })
    }

    fn list_provider_accounts(
        &self,
        kind: Option<String>,
        page: u32,
        per_page: u32,
    ) -> StoreFuture<'_, Page<ProviderAccount>> {
        Box::pin(async move {
            let offset = (page.saturating_sub(1) as i64) * (per_page as i64);
            let total = sqlx::query_scalar!(
                r#"
                SELECT COUNT(*) AS "total!"
                FROM ironflow.provider_accounts
                WHERE ($1::TEXT IS NULL OR kind = $1)
                "#,
                kind.as_deref(),
            )
            .fetch_one(&self.pool)
            .await
            .map_err(db_err)? as u64;
            let items = sqlx::query_as!(
                AccountRow,
                r#"
                SELECT id, name, display_name, kind, secret_key, enabled, priority, tags,
                    max_concurrency, alert_threshold, expires_at, plan, auth_failed_at,
                    created_by, created_at, updated_at
                FROM ironflow.provider_accounts
                WHERE ($1::TEXT IS NULL OR kind = $1)
                ORDER BY priority ASC, name ASC
                LIMIT $2 OFFSET $3
                "#,
                kind.as_deref(),
                per_page as i64,
                offset,
            )
            .fetch_all(&self.pool)
            .await
            .map_err(db_err)?
            .into_iter()
            .map(ProviderAccount::from)
            .collect();
            Ok(Page {
                items,
                total,
                page,
                per_page,
            })
        })
    }

    fn update_provider_account(
        &self,
        id: Uuid,
        update: ProviderAccountUpdate,
    ) -> StoreFuture<'_, ProviderAccount> {
        Box::pin(async move {
            let existing = self
                .get_provider_account(id)
                .await?
                .ok_or(StoreError::ProviderAccountNotFound(id))?;

            let renewed = update.enabled == Some(true) || update.auth_failed_at == Some(None);
            let display_name = update.display_name.unwrap_or(existing.display_name);
            let enabled = update.enabled.unwrap_or(existing.enabled);
            let priority = update.priority.unwrap_or(existing.priority);
            let tags = update.tags.unwrap_or(existing.tags);
            let max_concurrency = update
                .max_concurrency
                .unwrap_or(existing.max_concurrency)
                .map(|v| v as i32);
            let alert_threshold = update.alert_threshold.unwrap_or(existing.alert_threshold);
            let expires_at = update.expires_at.unwrap_or(existing.expires_at);
            let plan = update.plan.unwrap_or(existing.plan);
            let auth_failed_at = update.auth_failed_at.unwrap_or(existing.auth_failed_at);

            let row = sqlx::query_as!(
                AccountRow,
                r#"
                UPDATE ironflow.provider_accounts
                SET display_name = $2,
                    enabled = $3,
                    priority = $4,
                    tags = $5,
                    max_concurrency = $6,
                    alert_threshold = $7,
                    expires_at = $8,
                    plan = $9,
                    auth_failed_at = $10,
                    updated_at = NOW()
                WHERE id = $1
                RETURNING id, name, display_name, kind, secret_key, enabled, priority, tags,
                    max_concurrency, alert_threshold, expires_at, plan, auth_failed_at,
                    created_by, created_at, updated_at
                "#,
                id,
                &display_name,
                enabled,
                priority,
                &tags,
                max_concurrency,
                alert_threshold,
                expires_at,
                plan.as_deref(),
                auth_failed_at,
            )
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?
            .ok_or(StoreError::ProviderAccountNotFound(id))?;
            let account = ProviderAccount::from(row);
            if renewed && account.enabled {
                self.wake_capacity_sleepers(&ProviderKind::new(account.kind.as_str())).await?;
            }
            Ok(account)
        })
    }

    fn delete_provider_account(&self, id: Uuid) -> StoreFuture<'_, bool> {
        Box::pin(async move {
            // Windows and usage cascade; steps.account_id is set to NULL by the FK.
            let result = sqlx::query!("DELETE FROM ironflow.provider_accounts WHERE id = $1", id)
                .execute(&self.pool)
                .await
                .map_err(db_err)?;
            Ok(result.rows_affected() > 0)
        })
    }

    fn list_provider_account_windows(
        &self,
        ids: Vec<Uuid>,
    ) -> StoreFuture<'_, Vec<ProviderAccountWindow>> {
        Box::pin(async move { self.fetch_windows(&ids).await })
    }

    fn list_provider_account_usage(
        &self,
        id: Uuid,
        since: DateTime<Utc>,
    ) -> StoreFuture<'_, Vec<ProviderAccountUsagePoint>> {
        Box::pin(async move {
            sqlx::query_as!(
                ProviderAccountUsagePoint,
                r#"
                SELECT id, account_id, window_name AS "window",
                    NULLIF(model_scope, '') AS "model_scope?", utilization, resets_at,
                    status AS "status: AccountWindowStatus", observed_at
                FROM ironflow.provider_account_usage
                WHERE account_id = $1 AND observed_at >= $2
                ORDER BY observed_at ASC
                "#,
                id,
                since,
            )
            .fetch_all(&self.pool)
            .await
            .map_err(db_err)
        })
    }

    fn record_provider_account_observation(
        &self,
        id: Uuid,
        observation: NewProviderAccountObservation,
    ) -> StoreFuture<'_, Vec<ProviderAccountWindow>> {
        Box::pin(async move {
            let mut tx = self.pool.begin().await.map_err(db_err)?;

            let exists = sqlx::query_scalar!(
                "SELECT id FROM ironflow.provider_accounts WHERE id = $1 FOR UPDATE",
                id,
            )
            .fetch_optional(&mut *tx)
            .await
            .map_err(db_err)?;
            if exists.is_none() {
                return Err(StoreError::ProviderAccountNotFound(id));
            }

            let recorded = !observation.windows.is_empty();
            for window in &observation.windows {
                let scope = window.model_scope.as_deref().unwrap_or("");
                let status = window.status.to_string();
                // Out-of-order observations never overwrite a newer reading.
                sqlx::query!(
                    r#"
                    INSERT INTO ironflow.provider_account_windows
                        (account_id, window_name, model_scope, utilization, resets_at,
                         status, observed_at)
                    VALUES ($1, $2, $3, $4, $5, $6, $7)
                    ON CONFLICT (account_id, window_name, model_scope) DO UPDATE
                    SET utilization = EXCLUDED.utilization,
                        resets_at = EXCLUDED.resets_at,
                        status = EXCLUDED.status,
                        observed_at = EXCLUDED.observed_at,
                        updated_at = NOW()
                    WHERE EXCLUDED.observed_at >= ironflow.provider_account_windows.observed_at
                    "#,
                    id,
                    &window.window,
                    scope,
                    window.utilization,
                    window.resets_at,
                    &status,
                    window.observed_at,
                )
                .execute(&mut *tx)
                .await
                .map_err(db_err)?;

                sqlx::query!(
                    r#"
                    INSERT INTO ironflow.provider_account_usage
                        (id, account_id, window_name, model_scope, utilization, resets_at,
                         status, observed_at)
                    VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                    "#,
                    Uuid::now_v7(),
                    id,
                    &window.window,
                    scope,
                    window.utilization,
                    window.resets_at,
                    &status,
                    window.observed_at,
                )
                .execute(&mut *tx)
                .await
                .map_err(db_err)?;
            }

            if observation.auth_failed {
                sqlx::query!(
                    r#"
                    UPDATE ironflow.provider_accounts
                    SET auth_failed_at = NOW(), updated_at = NOW()
                    WHERE id = $1
                    "#,
                    id,
                )
                .execute(&mut *tx)
                .await
                .map_err(db_err)?;
            } else if recorded {
                sqlx::query!(
                    "UPDATE ironflow.provider_accounts SET auth_failed_at = NULL WHERE id = $1",
                    id,
                )
                .execute(&mut *tx)
                .await
                .map_err(db_err)?;
            }

            tx.commit().await.map_err(db_err)?;
            self.fetch_windows(&[id]).await
        })
    }

    fn purge_provider_account_usage(&self, before: DateTime<Utc>) -> StoreFuture<'_, u64> {
        Box::pin(async move {
            let result = sqlx::query!(
                "DELETE FROM ironflow.provider_account_usage WHERE observed_at < $1",
                before,
            )
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
            Ok(result.rows_affected())
        })
    }

    fn list_provider_account_candidates(
        &self,
        kind: String,
    ) -> StoreFuture<'_, Vec<ProviderAccountCandidate>> {
        Box::pin(async move {
            let accounts: Vec<ProviderAccount> = sqlx::query_as!(
                AccountRow,
                r#"
                SELECT id, name, display_name, kind, secret_key, enabled, priority, tags,
                    max_concurrency, alert_threshold, expires_at, plan, auth_failed_at,
                    created_by, created_at, updated_at
                FROM ironflow.provider_accounts
                WHERE kind = $1
                  AND enabled
                  AND auth_failed_at IS NULL
                  AND expires_at > NOW()
                ORDER BY priority ASC, name ASC
                "#,
                &kind,
            )
            .fetch_all(&self.pool)
            .await
            .map_err(db_err)?
            .into_iter()
            .map(ProviderAccount::from)
            .collect();

            if accounts.is_empty() {
                return Ok(Vec::new());
            }
            let ids: Vec<Uuid> = accounts.iter().map(|a| a.id).collect();

            let running_rows = sqlx::query!(
                r#"
                SELECT s.account_id as "account_id!", COUNT(*) as "running!: i64"
                FROM ironflow.steps s
                JOIN lib_fsm.state_machine sm ON sm.state_machine__id = s.state_machine__id
                JOIN lib_fsm.abstract_state ast ON ast.abstract_state__id = sm.abstract_state__id
                JOIN ironflow.runs r ON r.id = s.run_id
                WHERE s.account_id = ANY($1)
                  AND ast.name = 'running'
                  AND r.lease_expires_at > NOW()
                GROUP BY s.account_id
                "#,
                &ids,
            )
            .fetch_all(&self.pool)
            .await
            .map_err(db_err)?;
            let running: HashMap<Uuid, u32> = running_rows
                .into_iter()
                .map(|r| (r.account_id, r.running as u32))
                .collect();

            let mut windows_by_account: HashMap<Uuid, Vec<ProviderAccountWindow>> = HashMap::new();
            for window in self.fetch_windows(&ids).await? {
                windows_by_account
                    .entry(window.account_id)
                    .or_default()
                    .push(window);
            }

            Ok(accounts
                .into_iter()
                .map(|account| ProviderAccountCandidate {
                    windows: windows_by_account.remove(&account.id).unwrap_or_default(),
                    running_steps: running.get(&account.id).copied().unwrap_or(0),
                    account,
                })
                .collect())
        })
    }
}
