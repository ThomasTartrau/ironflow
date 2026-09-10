use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

use crate::entities::{NewSchedule, Page, Schedule, ScheduleSource, ScheduleUpdate};
use crate::error::StoreError;
use crate::schedule_store::ScheduleStore;
use crate::store::StoreFuture;

use super::PostgresStore;

struct ScheduleRow {
    id: Uuid,
    workflow_name: String,
    cron_expression: String,
    inputs: Value,
    source: String,
    disabled_at: Option<DateTime<Utc>>,
    last_triggered_at: Option<DateTime<Utc>>,
    next_trigger_at: Option<DateTime<Utc>>,
    created_by_user_id: Uuid,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<ScheduleRow> for Schedule {
    fn from(row: ScheduleRow) -> Self {
        Self {
            id: row.id,
            workflow_name: row.workflow_name,
            cron_expression: row.cron_expression,
            inputs: row.inputs,
            source: row.source.parse().unwrap_or(ScheduleSource::Api),
            disabled_at: row.disabled_at,
            last_triggered_at: row.last_triggered_at,
            next_trigger_at: row.next_trigger_at,
            created_by_user_id: row.created_by_user_id,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

struct ScheduleRowWithTotal {
    id: Uuid,
    workflow_name: String,
    cron_expression: String,
    inputs: Value,
    source: String,
    disabled_at: Option<DateTime<Utc>>,
    last_triggered_at: Option<DateTime<Utc>>,
    next_trigger_at: Option<DateTime<Utc>>,
    created_by_user_id: Uuid,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    total_count: i64,
}

impl From<ScheduleRowWithTotal> for Schedule {
    fn from(row: ScheduleRowWithTotal) -> Self {
        Self {
            id: row.id,
            workflow_name: row.workflow_name,
            cron_expression: row.cron_expression,
            inputs: row.inputs,
            source: row.source.parse().unwrap_or(ScheduleSource::Api),
            disabled_at: row.disabled_at,
            last_triggered_at: row.last_triggered_at,
            next_trigger_at: row.next_trigger_at,
            created_by_user_id: row.created_by_user_id,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

impl ScheduleStore for PostgresStore {
    fn create_schedule(&self, req: NewSchedule) -> StoreFuture<'_, Schedule> {
        Box::pin(async move {
            let id = Uuid::now_v7();
            let now = Utc::now();
            let source_str = req.source.as_str();
            let row = sqlx::query_as!(
                ScheduleRow,
                r#"
                INSERT INTO ironflow.schedules
                    (id, workflow_name, cron_expression, inputs, source,
                     next_trigger_at, created_by_user_id, created_at, updated_at)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                RETURNING id, workflow_name, cron_expression, inputs, source,
                    disabled_at, last_triggered_at, next_trigger_at,
                    created_by_user_id, created_at, updated_at
                "#,
                id,
                &req.workflow_name,
                &req.cron_expression,
                &req.inputs,
                source_str,
                req.next_trigger_at,
                req.created_by_user_id,
                now,
                now,
            )
            .fetch_one(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(Schedule::from(row))
        })
    }

    fn find_schedule_by_id(&self, id: Uuid) -> StoreFuture<'_, Option<Schedule>> {
        Box::pin(async move {
            let row = sqlx::query_as!(
                ScheduleRow,
                r#"
                SELECT id, workflow_name, cron_expression, inputs, source,
                    disabled_at, last_triggered_at, next_trigger_at,
                    created_by_user_id, created_at, updated_at
                FROM ironflow.schedules
                WHERE id = $1
                "#,
                id,
            )
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(row.map(Schedule::from))
        })
    }

    fn list_schedules(&self, page: u32, per_page: u32) -> StoreFuture<'_, Page<Schedule>> {
        Box::pin(async move {
            let offset = (page.saturating_sub(1) as i64) * (per_page as i64);
            let rows = sqlx::query_as!(
                ScheduleRowWithTotal,
                r#"
                SELECT id, workflow_name, cron_expression, inputs, source,
                    disabled_at, last_triggered_at, next_trigger_at,
                    created_by_user_id, created_at, updated_at,
                    COUNT(*) OVER () as "total_count!: i64"
                FROM ironflow.schedules
                ORDER BY created_at DESC
                LIMIT $1 OFFSET $2
                "#,
                per_page as i64,
                offset,
            )
            .fetch_all(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            let total = rows.first().map(|r| r.total_count as u64).unwrap_or(0);
            let items = rows.into_iter().map(Schedule::from).collect();

            Ok(Page {
                items,
                total,
                page,
                per_page,
            })
        })
    }

    fn update_schedule(&self, id: Uuid, update: ScheduleUpdate) -> StoreFuture<'_, Schedule> {
        Box::pin(async move {
            let existing = sqlx::query_as!(
                ScheduleRow,
                r#"
                SELECT id, workflow_name, cron_expression, inputs, source,
                    disabled_at, last_triggered_at, next_trigger_at,
                    created_by_user_id, created_at, updated_at
                FROM ironflow.schedules
                WHERE id = $1
                "#,
                id,
            )
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?
            .ok_or(StoreError::ScheduleNotFound(id))?;

            let cron_expression = update.cron_expression.unwrap_or(existing.cron_expression);
            let inputs = update.inputs.unwrap_or(existing.inputs);
            let disabled_at = match update.disabled_at {
                Some(v) => v,
                None => existing.disabled_at,
            };
            let next_trigger_at = match update.next_trigger_at {
                Some(v) => v,
                None => existing.next_trigger_at,
            };
            let last_triggered_at = match update.last_triggered_at {
                Some(v) => v,
                None => existing.last_triggered_at,
            };
            let now = Utc::now();

            let row = sqlx::query_as!(
                ScheduleRow,
                r#"
                UPDATE ironflow.schedules
                SET cron_expression = $2,
                    inputs = $3,
                    disabled_at = $4,
                    next_trigger_at = $5,
                    last_triggered_at = $6,
                    updated_at = $7
                WHERE id = $1
                RETURNING id, workflow_name, cron_expression, inputs, source,
                    disabled_at, last_triggered_at, next_trigger_at,
                    created_by_user_id, created_at, updated_at
                "#,
                id,
                &cron_expression,
                &inputs,
                disabled_at,
                next_trigger_at,
                last_triggered_at,
                now,
            )
            .fetch_one(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(Schedule::from(row))
        })
    }

    fn delete_schedule(&self, id: Uuid) -> StoreFuture<'_, ()> {
        Box::pin(async move {
            let result = sqlx::query!("DELETE FROM ironflow.schedules WHERE id = $1", id,)
                .execute(&self.pool)
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            if result.rows_affected() == 0 {
                return Err(StoreError::ScheduleNotFound(id));
            }
            Ok(())
        })
    }

    fn claim_due_schedules(&self) -> StoreFuture<'_, Vec<Schedule>> {
        Box::pin(async move {
            let rows = sqlx::query_as!(
                ScheduleRow,
                r#"
                WITH due AS (
                    SELECT id
                    FROM ironflow.schedules
                    WHERE disabled_at IS NULL
                      AND next_trigger_at IS NOT NULL
                      AND next_trigger_at <= NOW()
                    FOR UPDATE SKIP LOCKED
                )
                UPDATE ironflow.schedules s
                SET last_triggered_at = NOW(),
                    next_trigger_at = NULL,
                    updated_at = NOW()
                FROM due
                WHERE s.id = due.id
                RETURNING s.id, s.workflow_name, s.cron_expression,
                    s.inputs, s.source,
                    s.disabled_at, s.last_triggered_at, s.next_trigger_at,
                    s.created_by_user_id, s.created_at, s.updated_at
                "#,
            )
            .fetch_all(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(rows.into_iter().map(Schedule::from).collect())
        })
    }
}
