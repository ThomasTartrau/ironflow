use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{FromRow, query_as};
use uuid::Uuid;

use crate::entities::{
    NewSchedule, Page, Schedule, ScheduleFiring, ScheduleNext, ScheduleSource, ScheduleUpdate,
};
use crate::error::StoreError;
use crate::schedule_store::ScheduleStore;
use crate::store::StoreFuture;

use super::PostgresStore;
use super::run_store::insert_run;

#[derive(FromRow)]
struct ScheduleRow {
    id: Uuid,
    workflow_name: String,
    cron_expression: String,
    inputs: Value,
    source: String,
    disabled_at: Option<DateTime<Utc>>,
    last_triggered_at: Option<DateTime<Utc>>,
    next_trigger_at: Option<DateTime<Utc>>,
    last_error: Option<String>,
    priority: i16,
    created_by_user_id: Option<Uuid>,
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
            last_error: row.last_error,
            priority: row.priority,
            created_by_user_id: row.created_by_user_id,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[derive(FromRow)]
struct ScheduleRowWithTotal {
    id: Uuid,
    workflow_name: String,
    cron_expression: String,
    inputs: Value,
    source: String,
    disabled_at: Option<DateTime<Utc>>,
    last_triggered_at: Option<DateTime<Utc>>,
    next_trigger_at: Option<DateTime<Utc>>,
    last_error: Option<String>,
    priority: i16,
    created_by_user_id: Option<Uuid>,
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
            last_error: row.last_error,
            priority: row.priority,
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
            let row = query_as::<_, ScheduleRow>(
                r#"
                INSERT INTO ironflow.schedules
                    (id, workflow_name, cron_expression, inputs, source,
                     next_trigger_at, created_by_user_id, created_at, updated_at, priority)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
                RETURNING id, workflow_name, cron_expression, inputs, source,
                    disabled_at, last_triggered_at, next_trigger_at, last_error,
                    priority, created_by_user_id, created_at, updated_at
                "#,
            )
            .bind(id)
            .bind(&req.workflow_name)
            .bind(&req.cron_expression)
            .bind(&req.inputs)
            .bind(source_str)
            .bind(req.next_trigger_at)
            .bind(req.created_by_user_id)
            .bind(now)
            .bind(now)
            .bind(req.priority)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(Schedule::from(row))
        })
    }

    fn find_schedule_by_id(&self, id: Uuid) -> StoreFuture<'_, Option<Schedule>> {
        Box::pin(async move {
            let row = query_as::<_, ScheduleRow>(
                r#"
                SELECT id, workflow_name, cron_expression, inputs, source,
                    disabled_at, last_triggered_at, next_trigger_at, last_error,
                    priority, created_by_user_id, created_at, updated_at
                FROM ironflow.schedules
                WHERE id = $1
                "#,
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(row.map(Schedule::from))
        })
    }

    fn list_schedules(&self, page: u32, per_page: u32) -> StoreFuture<'_, Page<Schedule>> {
        Box::pin(async move {
            let offset = (page.saturating_sub(1) as i64) * (per_page as i64);
            let rows = query_as::<_, ScheduleRowWithTotal>(
                r#"
                SELECT id, workflow_name, cron_expression, inputs, source,
                    disabled_at, last_triggered_at, next_trigger_at, last_error,
                    priority, created_by_user_id, created_at, updated_at,
                    COUNT(*) OVER () AS total_count
                FROM ironflow.schedules
                ORDER BY created_at DESC
                LIMIT $1 OFFSET $2
                "#,
            )
            .bind(per_page as i64)
            .bind(offset)
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
            let existing = query_as::<_, ScheduleRow>(
                r#"
                SELECT id, workflow_name, cron_expression, inputs, source,
                    disabled_at, last_triggered_at, next_trigger_at, last_error,
                    priority, created_by_user_id, created_at, updated_at
                FROM ironflow.schedules
                WHERE id = $1
                "#,
            )
            .bind(id)
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
            let last_error = match update.last_error {
                Some(v) => v,
                None => existing.last_error,
            };
            let priority = update.priority.unwrap_or(existing.priority);
            let now = Utc::now();

            let row = query_as::<_, ScheduleRow>(
                r#"
                UPDATE ironflow.schedules
                SET cron_expression = $2,
                    inputs = $3,
                    disabled_at = $4,
                    next_trigger_at = $5,
                    last_triggered_at = $6,
                    last_error = $7,
                    updated_at = $8,
                    priority = $9
                WHERE id = $1
                RETURNING id, workflow_name, cron_expression, inputs, source,
                    disabled_at, last_triggered_at, next_trigger_at, last_error,
                    priority, created_by_user_id, created_at, updated_at
                "#,
            )
            .bind(id)
            .bind(&cron_expression)
            .bind(&inputs)
            .bind(disabled_at)
            .bind(next_trigger_at)
            .bind(last_triggered_at)
            .bind(last_error)
            .bind(now)
            .bind(priority)
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

    fn list_due_schedules(&self) -> StoreFuture<'_, Vec<Schedule>> {
        Box::pin(async move {
            let rows = query_as::<_, ScheduleRow>(
                r#"
                SELECT id, workflow_name, cron_expression, inputs, source,
                    disabled_at, last_triggered_at, next_trigger_at, last_error,
                    priority, created_by_user_id, created_at, updated_at
                FROM ironflow.schedules
                WHERE disabled_at IS NULL
                  AND next_trigger_at IS NOT NULL
                  AND next_trigger_at <= NOW()
                ORDER BY next_trigger_at
                "#,
            )
            .fetch_all(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(rows.into_iter().map(Schedule::from).collect())
        })
    }

    fn fire_due_schedule(
        &self,
        id: Uuid,
        occurrence: DateTime<Utc>,
        next: ScheduleNext,
    ) -> StoreFuture<'_, Option<ScheduleFiring>> {
        Box::pin(async move {
            let mut tx = self
                .pool
                .begin()
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            // SKIP LOCKED: an instance firing this occurrence holds the row, so
            // this one steps aside instead of waiting to find it fired.
            let row = query_as::<_, ScheduleRow>(
                r#"
                SELECT id, workflow_name, cron_expression, inputs, source,
                    disabled_at, last_triggered_at, next_trigger_at, last_error,
                    priority, created_by_user_id, created_at, updated_at
                FROM ironflow.schedules
                WHERE id = $1
                  AND disabled_at IS NULL
                  AND next_trigger_at = $2
                FOR UPDATE SKIP LOCKED
                "#,
            )
            .bind(id)
            .bind(occurrence)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            let Some(row) = row else {
                return Ok(None);
            };
            let schedule = Schedule::from(row);

            let mut new_run = schedule.new_run(None);
            new_run.idempotency_key = Some(Schedule::occurrence_key(id, occurrence));
            // Dropping `tx` on error rolls back the run: the schedule stays due.
            let run = insert_run(&mut tx, self.get_run_lifecycle_machine_id(), new_run).await?;

            let (next_trigger_at, error) = match next {
                ScheduleNext::At(at) => (Some(at), None),
                ScheduleNext::Disable { error } => (None, Some(error)),
            };
            let now = Utc::now();

            let row = query_as::<_, ScheduleRow>(
                r#"
                UPDATE ironflow.schedules
                SET last_triggered_at = $2,
                    updated_at = $2,
                    next_trigger_at = $3,
                    disabled_at = CASE WHEN $4::text IS NULL THEN disabled_at ELSE $2 END,
                    last_error = COALESCE($4::text, last_error)
                WHERE id = $1
                RETURNING id, workflow_name, cron_expression, inputs, source,
                    disabled_at, last_triggered_at, next_trigger_at, last_error,
                    priority, created_by_user_id, created_at, updated_at
                "#,
            )
            .bind(id)
            .bind(now)
            .bind(next_trigger_at)
            .bind(error)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            tx.commit()
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(Some(ScheduleFiring {
                schedule: Schedule::from(row),
                run,
            }))
        })
    }
}
