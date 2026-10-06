use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{FromRow, query_as};
use tracing::warn;
use uuid::Uuid;

use crate::entities::{
    CatchupPolicy, DEFAULT_CATCHUP_MAX, DEFAULT_CATCHUP_WINDOW_SECS, DEFAULT_TIMEZONE,
    NewSchedule, OverlapPolicy, Page, Schedule, ScheduleFiring, ScheduleFiringPlan, ScheduleNext,
    SchedulePolicy, ScheduleSource, ScheduleUpdate, ScheduledRun,
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
    catchup: String,
    catchup_max: i32,
    catchup_window_secs: i32,
    overlap: String,
    timezone: String,
    created_by_user_id: Option<Uuid>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

/// Read the policy columns of a schedule row. The CHECK constraints of the
/// columns make every fallback unreachable; each one is logged.
fn row_policy(
    id: Uuid,
    catchup: &str,
    catchup_max: i32,
    catchup_window_secs: i32,
    overlap: &str,
    timezone: &str,
) -> SchedulePolicy {
    SchedulePolicy {
        catchup: catchup.parse().unwrap_or_else(|e| {
            warn!(schedule_id = %id, catchup, error = %e, "unknown catchup policy, using the default");
            CatchupPolicy::default()
        }),
        catchup_max: u32::try_from(catchup_max).unwrap_or_else(|e| {
            warn!(schedule_id = %id, catchup_max, error = %e, "invalid catchup_max, using the default");
            DEFAULT_CATCHUP_MAX
        }),
        catchup_window_secs: u32::try_from(catchup_window_secs).unwrap_or_else(|e| {
            warn!(
                schedule_id = %id,
                catchup_window_secs,
                error = %e,
                "invalid catchup_window_secs, using the default"
            );
            DEFAULT_CATCHUP_WINDOW_SECS
        }),
        overlap: overlap.parse().unwrap_or_else(|e| {
            warn!(schedule_id = %id, overlap, error = %e, "unknown overlap policy, using the default");
            OverlapPolicy::default()
        }),
        timezone: timezone.parse().unwrap_or_else(|e| {
            warn!(schedule_id = %id, timezone, error = %e, "unknown timezone, using the default");
            DEFAULT_TIMEZONE
        }),
    }
}

impl From<ScheduleRow> for Schedule {
    fn from(row: ScheduleRow) -> Self {
        let policy = row_policy(
            row.id,
            &row.catchup,
            row.catchup_max,
            row.catchup_window_secs,
            &row.overlap,
            &row.timezone,
        );
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
            policy,
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
    catchup: String,
    catchup_max: i32,
    catchup_window_secs: i32,
    overlap: String,
    timezone: String,
    created_by_user_id: Option<Uuid>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    total_count: i64,
}

impl From<ScheduleRowWithTotal> for Schedule {
    fn from(row: ScheduleRowWithTotal) -> Self {
        Schedule::from(ScheduleRow {
            id: row.id,
            workflow_name: row.workflow_name,
            cron_expression: row.cron_expression,
            inputs: row.inputs,
            source: row.source,
            disabled_at: row.disabled_at,
            last_triggered_at: row.last_triggered_at,
            next_trigger_at: row.next_trigger_at,
            last_error: row.last_error,
            priority: row.priority,
            catchup: row.catchup,
            catchup_max: row.catchup_max,
            catchup_window_secs: row.catchup_window_secs,
            overlap: row.overlap,
            timezone: row.timezone,
            created_by_user_id: row.created_by_user_id,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

/// A `u32` policy bound converted to the `INTEGER` of its column.
fn policy_int(field: &str, value: u32) -> Result<i32, StoreError> {
    i32::try_from(value).map_err(|e| StoreError::Database(format!("{field} {value}: {e}")))
}

impl ScheduleStore for PostgresStore {
    fn create_schedule(&self, req: NewSchedule) -> StoreFuture<'_, Schedule> {
        Box::pin(async move {
            let id = Uuid::now_v7();
            let now = Utc::now();
            let source_str = req.source.as_str();
            let catchup_max = policy_int("catchup_max", req.policy.catchup_max)?;
            let catchup_window_secs =
                policy_int("catchup_window_secs", req.policy.catchup_window_secs)?;
            let row = query_as::<_, ScheduleRow>(
                r#"
                INSERT INTO ironflow.schedules
                    (id, workflow_name, cron_expression, inputs, source,
                     next_trigger_at, created_by_user_id, created_at, updated_at, priority,
                     catchup, catchup_max, catchup_window_secs, overlap, timezone)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
                RETURNING id, workflow_name, cron_expression, inputs, source,
                    disabled_at, last_triggered_at, next_trigger_at, last_error,
                    priority, catchup, catchup_max, catchup_window_secs, overlap, timezone,
                    created_by_user_id, created_at, updated_at
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
            .bind(req.policy.catchup.as_ref())
            .bind(catchup_max)
            .bind(catchup_window_secs)
            .bind(req.policy.overlap.as_ref())
            .bind(req.policy.timezone.name())
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
                    priority, catchup, catchup_max, catchup_window_secs, overlap, timezone,
                    created_by_user_id, created_at, updated_at
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
                    priority, catchup, catchup_max, catchup_window_secs, overlap, timezone,
                    created_by_user_id, created_at, updated_at,
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
                    priority, catchup, catchup_max, catchup_window_secs, overlap, timezone,
                    created_by_user_id, created_at, updated_at
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
            let existing_id = existing.id;
            let policy = update.policy.unwrap_or_else(|| {
                row_policy(
                    existing_id,
                    &existing.catchup,
                    existing.catchup_max,
                    existing.catchup_window_secs,
                    &existing.overlap,
                    &existing.timezone,
                )
            });
            let catchup_max = policy_int("catchup_max", policy.catchup_max)?;
            let catchup_window_secs =
                policy_int("catchup_window_secs", policy.catchup_window_secs)?;
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
                    priority = $9,
                    catchup = $10,
                    catchup_max = $11,
                    catchup_window_secs = $12,
                    overlap = $13,
                    timezone = $14
                WHERE id = $1
                RETURNING id, workflow_name, cron_expression, inputs, source,
                    disabled_at, last_triggered_at, next_trigger_at, last_error,
                    priority, catchup, catchup_max, catchup_window_secs, overlap, timezone,
                    created_by_user_id, created_at, updated_at
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
            .bind(policy.catchup.as_ref())
            .bind(catchup_max)
            .bind(catchup_window_secs)
            .bind(policy.overlap.as_ref())
            .bind(policy.timezone.name())
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
                    priority, catchup, catchup_max, catchup_window_secs, overlap, timezone,
                    created_by_user_id, created_at, updated_at
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
        due: DateTime<Utc>,
        plan: ScheduleFiringPlan,
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
                    priority, catchup, catchup_max, catchup_window_secs, overlap, timezone,
                    created_by_user_id, created_at, updated_at
                FROM ironflow.schedules
                WHERE id = $1
                  AND disabled_at IS NULL
                  AND next_trigger_at = $2
                FOR UPDATE SKIP LOCKED
                "#,
            )
            .bind(id)
            .bind(due)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            let Some(row) = row else {
                return Ok(None);
            };
            let schedule = Schedule::from(row);

            let machine_id = self.get_run_lifecycle_machine_id();
            let mut runs = Vec::with_capacity(plan.occurrences.len());
            let mut overlapped = Vec::new();
            for occurrence in plan.occurrences {
                let mut new_run = schedule.new_run(Some(occurrence), None);
                new_run.idempotency_key = Some(Schedule::occurrence_key(id, occurrence));
                // A concurrency conflict is a returned error, not an SQL one:
                // the transaction stays usable. Any other error drops `tx`,
                // which rolls back every run: the schedule stays due.
                match insert_run(&mut tx, machine_id, new_run).await {
                    Ok(run) => runs.push(ScheduledRun { occurrence, run }),
                    Err(StoreError::ConcurrencyConflict { .. }) => overlapped.push(occurrence),
                    Err(e) => return Err(e),
                }
            }
            let fired_any = !runs.is_empty();

            let (next_trigger_at, error) = match plan.next {
                ScheduleNext::At(at) => (Some(at), None),
                ScheduleNext::Disable { error } => (None, Some(error)),
            };
            let now = Utc::now();

            let row = query_as::<_, ScheduleRow>(
                r#"
                UPDATE ironflow.schedules
                SET last_triggered_at = CASE WHEN $5::boolean THEN $2 ELSE last_triggered_at END,
                    updated_at = $2,
                    next_trigger_at = $3,
                    disabled_at = CASE WHEN $4::text IS NULL THEN disabled_at ELSE $2 END,
                    last_error = COALESCE($4::text, last_error)
                WHERE id = $1
                RETURNING id, workflow_name, cron_expression, inputs, source,
                    disabled_at, last_triggered_at, next_trigger_at, last_error,
                    priority, catchup, catchup_max, catchup_window_secs, overlap, timezone,
                    created_by_user_id, created_at, updated_at
                "#,
            )
            .bind(id)
            .bind(now)
            .bind(next_trigger_at)
            .bind(error)
            .bind(fired_any)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            tx.commit()
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(Some(ScheduleFiring {
                schedule: Schedule::from(row),
                runs,
                overlapped,
            }))
        })
    }
}
