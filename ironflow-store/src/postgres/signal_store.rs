//! PostgreSQL [`SignalStore`] implementation.
//!
//! Resolving a waiting step and suspending a run both lock the step row first,
//! then the run row: a delivery racing the run that opens the wait, or racing
//! a timeout, is serialized on the step and resolves it exactly once.

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::postgres::PgRow;
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;

use crate::entities::{
    NewSignal, Page, RunStatus, Signal, SignalFilter, SignalInsert, SignalStepResolution, Step,
    StepStatus,
};
use crate::error::StoreError;
use crate::signal_store::SignalStore;
use crate::store::StoreFuture;

use super::PostgresStore;
use super::helpers::{parse_run_status, parse_step_status, row_to_step};

/// Columns of `ironflow.signals`, in [`row_to_signal`] order.
const SIGNAL_COLUMNS: &str = "id, name, key, payload, idempotency_id, received_at";

/// Maximum number of signals returned by [`SignalStore::list_signals_for_key`].
const SIGNALS_FOR_KEY_LIMIT: i64 = 100;

/// FSM event moving a run from `sleeping` to `pending` on a delivery.
const SIGNAL_RECEIVED_EVENT: &str = "signal_received";

fn row_to_signal(row: &PgRow) -> Signal {
    Signal {
        id: row.get("id"),
        name: row.get("name"),
        key: row.get("key"),
        payload: row.get("payload"),
        idempotency_id: row.get("idempotency_id"),
        received_at: row.get("received_at"),
    }
}

/// A step row locked for the rest of the transaction.
struct LockedStep {
    run_id: Uuid,
    state_machine_id: Uuid,
    waiting: bool,
    output: Option<Value>,
}

/// Lock a step row and report whether it is a `signal` step still waiting.
async fn lock_step(
    tx: &mut Transaction<'_, Postgres>,
    step_id: Uuid,
) -> Result<LockedStep, StoreError> {
    let row = sqlx::query(
        r#"
        SELECT s.run_id, s.kind, s.output, s.state_machine__id, ast.name as state_name
        FROM ironflow.steps s
        JOIN lib_fsm.state_machine sm ON sm.state_machine__id = s.state_machine__id
        JOIN lib_fsm.abstract_state ast ON ast.abstract_state__id = sm.abstract_state__id
        WHERE s.id = $1
        FOR UPDATE OF s, sm
        "#,
    )
    .bind(step_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e| StoreError::Database(e.to_string()))?
    .ok_or(StoreError::StepNotFound(step_id))?;

    let kind: String = row.get("kind");
    let state_name: String = row.get("state_name");
    let status = parse_step_status(&state_name)?;

    Ok(LockedStep {
        run_id: row.get("run_id"),
        state_machine_id: row.get("state_machine__id"),
        waiting: kind == "signal" && status == StepStatus::Running,
        output: row.get("output"),
    })
}

/// Lock a run row and return its current status and state machine ID.
async fn lock_run(
    tx: &mut Transaction<'_, Postgres>,
    run_id: Uuid,
) -> Result<(RunStatus, Uuid), StoreError> {
    let row = sqlx::query(
        r#"
        SELECT r.state_machine__id, ast.name as state_name
        FROM ironflow.runs r
        JOIN lib_fsm.state_machine sm ON sm.state_machine__id = r.state_machine__id
        JOIN lib_fsm.abstract_state ast ON ast.abstract_state__id = sm.abstract_state__id
        WHERE r.id = $1
        FOR UPDATE OF r, sm
        "#,
    )
    .bind(run_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e| StoreError::Database(e.to_string()))?
    .ok_or(StoreError::RunNotFound(run_id))?;

    let state_name: String = row.get("state_name");
    Ok((parse_run_status(&state_name)?, row.get("state_machine__id")))
}

/// Fire an FSM event on a state machine instance.
async fn transition(
    tx: &mut Transaction<'_, Postgres>,
    state_machine_id: Uuid,
    event: &str,
) -> Result<(), StoreError> {
    sqlx::query("SELECT lib_fsm.state_machine_transition($1, $2)")
        .bind(state_machine_id)
        .bind(event)
        .fetch_one(&mut **tx)
        .await
        .map_err(|e| StoreError::Database(e.to_string()))?;
    Ok(())
}

impl SignalStore for PostgresStore {
    fn insert_signal(&self, signal: NewSignal) -> StoreFuture<'_, SignalInsert> {
        Box::pin(async move {
            let inserted = sqlx::query(&format!(
                r#"
                INSERT INTO ironflow.signals (id, name, key, payload, idempotency_id, received_at)
                VALUES ($1, $2, $3, $4, $5, NOW())
                ON CONFLICT (idempotency_id) DO NOTHING
                RETURNING {SIGNAL_COLUMNS}
                "#
            ))
            .bind(Uuid::now_v7())
            .bind(&signal.name)
            .bind(&signal.key)
            .bind(&signal.payload)
            .bind(&signal.idempotency_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            if let Some(row) = inserted {
                return Ok(SignalInsert::Created(row_to_signal(&row)));
            }

            // Only an idempotency conflict skips the insert.
            let idempotency_id = signal
                .idempotency_id
                .ok_or_else(|| StoreError::Database("signal insert returned no row".to_string()))?;
            let row = sqlx::query(&format!(
                "SELECT {SIGNAL_COLUMNS} FROM ironflow.signals WHERE idempotency_id = $1"
            ))
            .bind(&idempotency_id)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(SignalInsert::Duplicate(row_to_signal(&row)))
        })
    }

    fn list_signals(
        &self,
        filter: SignalFilter,
        page: u32,
        per_page: u32,
    ) -> StoreFuture<'_, Page<Signal>> {
        Box::pin(async move {
            let page = page.max(1);
            let per_page = per_page.clamp(1, 100);

            let mut conditions = Vec::new();
            let mut bind_idx = 1u32;
            if filter.name.is_some() {
                conditions.push(format!("name = ${bind_idx}"));
                bind_idx += 1;
            }
            if filter.key.is_some() {
                conditions.push(format!("key = ${bind_idx}"));
                bind_idx += 1;
            }
            let where_clause = if conditions.is_empty() {
                String::new()
            } else {
                format!("WHERE {}", conditions.join(" AND "))
            };

            let count_sql = format!("SELECT COUNT(*) FROM ironflow.signals {where_clause}");
            let mut count_query = sqlx::query_scalar::<_, i64>(&count_sql);
            if let Some(ref name) = filter.name {
                count_query = count_query.bind(name);
            }
            if let Some(ref key) = filter.key {
                count_query = count_query.bind(key);
            }
            let total = count_query
                .fetch_one(&self.pool)
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            let items_sql = format!(
                "SELECT {SIGNAL_COLUMNS} FROM ironflow.signals {where_clause} \
                 ORDER BY received_at DESC, id DESC LIMIT ${bind_idx} OFFSET ${}",
                bind_idx + 1
            );
            let mut items_query = sqlx::query(&items_sql);
            if let Some(ref name) = filter.name {
                items_query = items_query.bind(name);
            }
            if let Some(ref key) = filter.key {
                items_query = items_query.bind(key);
            }
            let rows = items_query
                .bind(i64::from(per_page))
                .bind(i64::from(page - 1) * i64::from(per_page))
                .fetch_all(&self.pool)
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(Page {
                items: rows.iter().map(row_to_signal).collect(),
                total: total as u64,
                page,
                per_page,
            })
        })
    }

    fn list_signals_for_key(
        &self,
        name: &str,
        key: &str,
        since: DateTime<Utc>,
    ) -> StoreFuture<'_, Vec<Signal>> {
        let name = name.to_string();
        let key = key.to_string();
        Box::pin(async move {
            let rows = sqlx::query(&format!(
                "SELECT {SIGNAL_COLUMNS} FROM ironflow.signals \
                 WHERE name = $1 AND key = $2 AND received_at >= $3 \
                 ORDER BY received_at ASC, id ASC LIMIT $4"
            ))
            .bind(&name)
            .bind(&key)
            .bind(since)
            .bind(SIGNALS_FOR_KEY_LIMIT)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(rows.iter().map(row_to_signal).collect())
        })
    }

    fn list_signal_waiters(&self, name: &str, key: &str) -> StoreFuture<'_, Vec<Step>> {
        let name = name.to_string();
        let key = key.to_string();
        Box::pin(async move {
            let rows = sqlx::query(
                r#"
                SELECT s.*, ast.name as state_name
                FROM ironflow.steps s
                JOIN lib_fsm.state_machine sm ON sm.state_machine__id = s.state_machine__id
                JOIN lib_fsm.abstract_state ast ON ast.abstract_state__id = sm.abstract_state__id
                JOIN ironflow.runs r ON r.id = s.run_id
                JOIN lib_fsm.state_machine rsm ON rsm.state_machine__id = r.state_machine__id
                JOIN lib_fsm.abstract_state rast ON rast.abstract_state__id = rsm.abstract_state__id
                WHERE s.kind = 'signal'
                  AND s.input->>'name' = $1
                  AND s.input->>'key' = $2
                  AND ast.name = 'running'
                  AND rast.name IN ('sleeping', 'running', 'pending', 'paused')
                ORDER BY s.created_at ASC, s.id ASC
                "#,
            )
            .bind(&name)
            .bind(&key)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            rows.iter().map(row_to_step).collect()
        })
    }

    fn resolve_signal_step(
        &self,
        step_id: Uuid,
        output: Value,
    ) -> StoreFuture<'_, SignalStepResolution> {
        Box::pin(async move {
            let mut tx = self
                .pool
                .begin()
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            let step = lock_step(&mut tx, step_id).await?;
            if !step.waiting {
                tx.commit()
                    .await
                    .map_err(|e| StoreError::Database(e.to_string()))?;
                return Ok(SignalStepResolution::NotWaiting {
                    output: step.output,
                });
            }

            let event =
                PostgresStore::step_status_to_event(StepStatus::Running, StepStatus::Completed)?;
            transition(&mut tx, step.state_machine_id, event).await?;
            sqlx::query(
                r#"
                UPDATE ironflow.steps
                SET output = $1,
                    completed_at = NOW(),
                    duration_ms = COALESCE(
                        FLOOR(EXTRACT(EPOCH FROM (NOW() - started_at)) * 1000)::BIGINT, 0
                    ),
                    updated_at = NOW()
                WHERE id = $2
                "#,
            )
            .bind(&output)
            .bind(step_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            let (run_status, run_state_machine_id) = lock_run(&mut tx, step.run_id).await?;
            let run_resumed = run_status == RunStatus::Sleeping;
            if run_resumed {
                transition(&mut tx, run_state_machine_id, SIGNAL_RECEIVED_EVENT).await?;
                sqlx::query(
                    r#"
                    UPDATE ironflow.runs
                    SET scheduled_at = NULL, capacity_wait_kind = NULL, updated_at = NOW()
                    WHERE id = $1
                    "#,
                )
                .bind(step.run_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;
            } else if run_status == RunStatus::Paused {
                // The signal ended the wait of a run paused while sleeping: the
                // resume requeues it instead of putting it back to sleep.
                sqlx::query(
                    r#"
                    UPDATE ironflow.runs
                    SET resume_status = 'pending', scheduled_at = NULL,
                        capacity_wait_kind = NULL, updated_at = NOW()
                    WHERE id = $1 AND resume_status = 'sleeping'
                    "#,
                )
                .bind(step.run_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;
            }

            tx.commit()
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(SignalStepResolution::Resolved {
                run_id: step.run_id,
                run_resumed,
            })
        })
    }

    fn suspend_run_on_signal(
        &self,
        run_id: Uuid,
        step_id: Uuid,
        deadline_at: DateTime<Utc>,
    ) -> StoreFuture<'_, bool> {
        Box::pin(async move {
            let mut tx = self
                .pool
                .begin()
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            let step = lock_step(&mut tx, step_id).await?;
            let (run_status, run_state_machine_id) = lock_run(&mut tx, run_id).await?;
            if run_status != RunStatus::Running {
                return Err(StoreError::InvalidTransition {
                    from: run_status,
                    to: RunStatus::Sleeping,
                });
            }

            let event =
                PostgresStore::run_status_to_event(RunStatus::Running, RunStatus::Sleeping)?;
            transition(&mut tx, run_state_machine_id, event).await?;

            // A signal that resolved the step before the run could sleep left
            // nothing to wait for: the next waker tick resumes it right away.
            let scheduled_at = if step.waiting {
                deadline_at
            } else {
                Utc::now()
            };
            sqlx::query(
                r#"
                UPDATE ironflow.runs
                SET scheduled_at = $1,
                    worker_id = NULL,
                    lease_expires_at = NULL,
                    capacity_wait_kind = NULL,
                    updated_at = NOW()
                WHERE id = $2
                "#,
            )
            .bind(scheduled_at)
            .bind(run_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            tx.commit()
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(step.waiting)
        })
    }

    fn purge_signals(&self, before: DateTime<Utc>) -> StoreFuture<'_, u64> {
        Box::pin(async move {
            let result = sqlx::query("DELETE FROM ironflow.signals WHERE received_at < $1")
                .bind(before)
                .execute(&self.pool)
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;
            Ok(result.rows_affected())
        })
    }
}
