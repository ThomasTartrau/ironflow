use chrono::Utc;
use sqlx::Row;
use uuid::Uuid;

use crate::entities::{
    NewRun, NewStep, NewStepDependency, Page, Run, RunFilter, RunStats, RunStatus, RunUpdate, Step,
    StepDependency, StepUpdate,
};
use crate::error::StoreError;
use crate::store::{RunStore, StoreFuture};

use super::PostgresStore;
use super::helpers::{row_to_run, row_to_step, run_status_to_db_str};

/// Build SQL WHERE conditions from a [`RunFilter`], returning `(where_clause, next_bind_idx)`.
fn build_run_filter_conditions(filter: &RunFilter) -> (String, u32) {
    let mut conditions = Vec::new();
    let mut bind_idx = 1u32;

    if filter.workflow_name.is_some() {
        conditions.push(format!("r.workflow_name ILIKE ${bind_idx}"));
        bind_idx += 1;
    }
    if filter.status.is_some() {
        conditions.push(format!("ast.name = ${bind_idx}"));
        bind_idx += 1;
    }
    if filter.created_after.is_some() {
        conditions.push(format!("r.created_at >= ${bind_idx}"));
        bind_idx += 1;
    }
    if filter.created_before.is_some() {
        conditions.push(format!("r.created_at <= ${bind_idx}"));
        bind_idx += 1;
    }
    if filter.labels.is_some() {
        conditions.push(format!("r.labels @> ${bind_idx}"));
        bind_idx += 1;
    }
    if let Some(has_steps) = filter.has_steps {
        let steps_condition = if has_steps {
            "EXISTS (SELECT 1 FROM ironflow.steps s WHERE s.run_id = r.id)"
        } else {
            "NOT EXISTS (SELECT 1 FROM ironflow.steps s WHERE s.run_id = r.id)"
        };
        conditions.push(format!(
            "(ast.name NOT IN (${bind_idx}, ${}) OR {steps_condition})",
            bind_idx + 1
        ));
        bind_idx += 2;
    }

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    };

    (where_clause, bind_idx)
}

/// Bind [`RunFilter`] parameter values onto a dynamic SQL query.
fn bind_run_filter_params<'q>(
    mut query: sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>,
    filter: &'q RunFilter,
) -> sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments> {
    if let Some(ref wf) = filter.workflow_name {
        query = query.bind(format!("%{wf}%"));
    }
    if let Some(ref status) = filter.status {
        query = query.bind(run_status_to_db_str(status));
    }
    if let Some(after) = filter.created_after {
        query = query.bind(after);
    }
    if let Some(before) = filter.created_before {
        query = query.bind(before);
    }
    if let Some(ref labels) = filter.labels {
        query = query.bind(serde_json::to_value(labels).unwrap_or_default());
    }
    if filter.has_steps.is_some() {
        query = query.bind(run_status_to_db_str(&RunStatus::Completed));
        query = query.bind(run_status_to_db_str(&RunStatus::Cancelled));
    }
    query
}

impl RunStore for PostgresStore {
    fn create_run(&self, req: NewRun) -> StoreFuture<'_, Run> {
        Box::pin(async move {
            let id = Uuid::now_v7();
            let now = Utc::now();
            let trigger_json = serde_json::to_value(&req.trigger)?;

            // Get cached run_lifecycle FSM abstract machine ID
            let fsm_machine_id = self.get_run_lifecycle_machine_id();

            let mut tx = self
                .pool
                .begin()
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            // Create FSM instance at initial state (pending)
            let row = sqlx::query("SELECT lib_fsm.state_machine_create($1) as state_machine__id")
                .bind(fsm_machine_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(|e| StoreError::Database(format!("failed to create FSM instance: {e}")))?;
            let state_machine_id: Uuid = row.get("state_machine__id");

            // Insert run with FSM reference
            sqlx::query(
                r#"
                INSERT INTO ironflow.runs (id, workflow_name, state_machine__id, trigger, payload, max_retries, handler_version, labels, scheduled_at, created_at, updated_at)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
                "#,
            )
            .bind(id)
            .bind(&req.workflow_name)
            .bind(state_machine_id)
            .bind(&trigger_json)
            .bind(&req.payload)
            .bind(req.max_retries as i32)
            .bind(&req.handler_version)
            .bind(serde_json::to_value(&req.labels).unwrap_or_default())
            .bind(req.scheduled_at)
            .bind(now)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            // Fetch the inserted run with FSM state
            let row = sqlx::query(
                r#"
                SELECT r.*, ast.name as state_name
                FROM ironflow.runs r
                JOIN lib_fsm.state_machine sm ON sm.state_machine__id = r.state_machine__id
                JOIN lib_fsm.abstract_state ast ON ast.abstract_state__id = sm.abstract_state__id
                WHERE r.id = $1
                "#,
            )
            .bind(id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            let run = row_to_run(&row)?;

            tx.commit()
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(run)
        })
    }

    fn get_run(&self, id: Uuid) -> StoreFuture<'_, Option<Run>> {
        Box::pin(async move {
            let row = sqlx::query(
                r#"
                SELECT r.*, ast.name as state_name
                FROM ironflow.runs r
                JOIN lib_fsm.state_machine sm ON sm.state_machine__id = r.state_machine__id
                JOIN lib_fsm.abstract_state ast ON ast.abstract_state__id = sm.abstract_state__id
                WHERE r.id = $1
                "#,
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            row.map(|r| row_to_run(&r)).transpose()
        })
    }

    fn list_runs(&self, filter: RunFilter, page: u32, per_page: u32) -> StoreFuture<'_, Page<Run>> {
        Box::pin(async move {
            let page = page.max(1);
            let per_page = per_page.clamp(1, 100);
            let offset = ((page - 1) * per_page) as i64;

            let (where_clause, bind_idx) = build_run_filter_conditions(&filter);

            let limit_idx = bind_idx;
            let offset_idx = bind_idx + 1;
            let data_sql = format!(
                r#"
                SELECT r.*, ast.name as state_name, COUNT(*) OVER() as total_count
                FROM ironflow.runs r
                JOIN lib_fsm.state_machine sm ON sm.state_machine__id = r.state_machine__id
                JOIN lib_fsm.abstract_state ast ON ast.abstract_state__id = sm.abstract_state__id
                {where_clause}
                ORDER BY r.created_at DESC
                LIMIT ${limit_idx} OFFSET ${offset_idx}
                "#,
            );

            let data_query = bind_run_filter_params(sqlx::query(&data_sql), &filter)
                .bind(per_page as i64)
                .bind(offset);

            let rows = data_query
                .fetch_all(&self.pool)
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            // Extract total_count from first row (all rows have same value due to OVER())
            let total = if rows.is_empty() {
                0u64
            } else {
                rows[0].get::<i64, _>("total_count") as u64
            };

            let items: Result<Vec<Run>, StoreError> = rows.iter().map(row_to_run).collect();

            Ok(Page {
                items: items?,
                total,
                page,
                per_page,
            })
        })
    }

    fn update_run_status(&self, id: Uuid, new_status: RunStatus) -> StoreFuture<'_, ()> {
        Box::pin(async move {
            let mut tx = self
                .pool
                .begin()
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            // Get current state and state_machine__id in one query
            let row = sqlx::query(
                r#"
                SELECT ast.name as state_name, r.state_machine__id
                FROM ironflow.runs r
                JOIN lib_fsm.state_machine sm ON sm.state_machine__id = r.state_machine__id
                JOIN lib_fsm.abstract_state ast ON ast.abstract_state__id = sm.abstract_state__id
                WHERE r.id = $1
                FOR UPDATE
                "#,
            )
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?
            .ok_or(StoreError::RunNotFound(id))?;

            let current_state_name: &str = row.get("state_name");
            let current = super::helpers::parse_run_status(current_state_name)?;

            if !current.can_transition_to(&new_status) {
                return Err(StoreError::InvalidTransition {
                    from: current,
                    to: new_status,
                });
            }

            if current == new_status && new_status.is_terminal() {
                return Ok(());
            }

            let event = PostgresStore::run_status_to_event(current, new_status)?;
            let now = Utc::now();

            let state_machine_id: Uuid = row.get("state_machine__id");

            // Perform FSM transition
            sqlx::query("SELECT lib_fsm.state_machine_transition($1, $2)")
                .bind(state_machine_id)
                .bind(event)
                .fetch_one(&mut *tx)
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            // Update timestamps with dynamic bind indices
            let mut sql = String::from("UPDATE ironflow.runs SET updated_at = $1");
            let mut bind_idx = 2u32;
            if new_status == RunStatus::Running {
                sql.push_str(&format!(", started_at = COALESCE(started_at, ${bind_idx})"));
                bind_idx += 1;
            }
            if new_status.is_terminal() {
                sql.push_str(&format!(
                    ", completed_at = COALESCE(completed_at, ${bind_idx})"
                ));
                bind_idx += 1;
            }
            sql.push_str(&format!(" WHERE id = ${bind_idx}"));

            let mut query = sqlx::query(&sql).bind(now);
            if new_status == RunStatus::Running {
                query = query.bind(now);
            }
            if new_status.is_terminal() {
                query = query.bind(now);
            }
            query = query.bind(id);

            query
                .execute(&mut *tx)
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            tx.commit()
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(())
        })
    }

    fn update_run(&self, id: Uuid, update: RunUpdate) -> StoreFuture<'_, ()> {
        Box::pin(async move {
            let mut tx = self
                .pool
                .begin()
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            PostgresStore::apply_run_update(&mut tx, id, &update).await?;

            tx.commit()
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(())
        })
    }

    fn update_run_returning(&self, id: Uuid, update: RunUpdate) -> StoreFuture<'_, Run> {
        Box::pin(async move {
            let mut tx = self
                .pool
                .begin()
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            PostgresStore::apply_run_update(&mut tx, id, &update).await?;

            let row = sqlx::query(
                r#"
                SELECT r.*, ast.name as state_name
                FROM ironflow.runs r
                JOIN lib_fsm.state_machine sm ON sm.state_machine__id = r.state_machine__id
                JOIN lib_fsm.abstract_state ast ON ast.abstract_state__id = sm.abstract_state__id
                WHERE r.id = $1
                "#,
            )
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?
            .ok_or(StoreError::RunNotFound(id))?;

            let run = row_to_run(&row)?;

            tx.commit()
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(run)
        })
    }

    fn pick_next_pending(&self) -> StoreFuture<'_, Option<Run>> {
        Box::pin(async move {
            let now = Utc::now();
            let event = "picked_up";

            let mut tx = self
                .pool
                .begin()
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            // Find a pending run and transition it
            let run_row = sqlx::query(
                r#"
                SELECT r.id, r.state_machine__id
                FROM ironflow.runs r
                JOIN lib_fsm.state_machine sm ON sm.state_machine__id = r.state_machine__id
                JOIN lib_fsm.abstract_state ast ON ast.abstract_state__id = sm.abstract_state__id
                WHERE ast.name = 'pending'
                  AND (r.scheduled_at IS NULL OR r.scheduled_at <= NOW())
                ORDER BY r.created_at ASC
                LIMIT 1
                FOR UPDATE SKIP LOCKED
                "#,
            )
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            if let Some(run_row) = run_row {
                let run_id: Uuid = run_row.get("id");
                let state_machine_id: Uuid = run_row.get("state_machine__id");

                // Perform FSM transition
                sqlx::query("SELECT lib_fsm.state_machine_transition($1, $2)")
                    .bind(state_machine_id)
                    .bind(event)
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(|e| StoreError::Database(e.to_string()))?;

                // Update timestamps
                sqlx::query(
                    "UPDATE ironflow.runs SET started_at = COALESCE(started_at, $1), updated_at = $1 WHERE id = $2",
                )
                .bind(now)
                .bind(run_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

                // Fetch and return the updated run
                let updated_row = sqlx::query(
                    r#"
                    SELECT r.*, ast.name as state_name
                    FROM ironflow.runs r
                    JOIN lib_fsm.state_machine sm ON sm.state_machine__id = r.state_machine__id
                    JOIN lib_fsm.abstract_state ast ON ast.abstract_state__id = sm.abstract_state__id
                    WHERE r.id = $1
                    "#
                )
                .bind(run_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

                let run = row_to_run(&updated_row)?;

                tx.commit()
                    .await
                    .map_err(|e| StoreError::Database(e.to_string()))?;

                return Ok(Some(run));
            }

            Ok(None)
        })
    }

    fn create_step(&self, req: NewStep) -> StoreFuture<'_, Step> {
        Box::pin(async move {
            let id = Uuid::now_v7();
            let now = Utc::now();

            // Get cached step_lifecycle FSM abstract machine ID
            let fsm_machine_id = self.get_step_lifecycle_machine_id();

            let mut tx = self
                .pool
                .begin()
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            // Create FSM instance at initial state (pending)
            let row = sqlx::query("SELECT lib_fsm.state_machine_create($1) as state_machine__id")
                .bind(fsm_machine_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(|e| StoreError::Database(format!("failed to create FSM instance: {e}")))?;
            let state_machine_id: Uuid = row.get("state_machine__id");

            let kind_str = super::helpers::step_kind_to_str(&req.kind);
            sqlx::query(
                r#"
                INSERT INTO ironflow.steps (id, run_id, name, kind, position, state_machine__id, input, created_at, updated_at)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                "#,
            )
            .bind(id)
            .bind(req.run_id)
            .bind(&req.name)
            .bind(kind_str.as_ref())
            .bind(req.position as i32)
            .bind(state_machine_id)
            .bind(req.input.as_ref())
            .bind(now)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(|e| {
                // Map foreign key constraint errors to RunNotFound
                if e.to_string().contains("foreign key") || e.to_string().contains("run_id") {
                    StoreError::RunNotFound(req.run_id)
                } else {
                    StoreError::Database(e.to_string())
                }
            })?;

            // Fetch the inserted step with FSM state
            let row = sqlx::query(
                r#"
                SELECT s.*, ast.name as state_name
                FROM ironflow.steps s
                JOIN lib_fsm.state_machine sm ON sm.state_machine__id = s.state_machine__id
                JOIN lib_fsm.abstract_state ast ON ast.abstract_state__id = sm.abstract_state__id
                WHERE s.id = $1
                "#,
            )
            .bind(id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            let step = row_to_step(&row)?;

            tx.commit()
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(step)
        })
    }

    fn update_step(&self, id: Uuid, update: StepUpdate) -> StoreFuture<'_, ()> {
        Box::pin(async move {
            let mut tx = self
                .pool
                .begin()
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            // Handle status transition if provided
            if let Some(new_status) = update.status {
                let row = sqlx::query(
                    r#"
                    SELECT ast.name as state_name, s.state_machine__id
                    FROM ironflow.steps s
                    JOIN lib_fsm.state_machine sm ON sm.state_machine__id = s.state_machine__id
                    JOIN lib_fsm.abstract_state ast ON ast.abstract_state__id = sm.abstract_state__id
                    WHERE s.id = $1
                    FOR UPDATE
                    "#
                )
                .bind(id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?
                .ok_or(StoreError::StepNotFound(id))?;

                let current_state_name: &str = row.get("state_name");
                let current = super::helpers::parse_step_status(current_state_name)?;

                let event =
                    PostgresStore::step_status_to_event(current, new_status).map_err(|_| {
                        StoreError::Database(format!(
                            "invalid step status transition from {:?} to {:?}",
                            current, new_status
                        ))
                    })?;

                let state_machine_id: Uuid = row.get("state_machine__id");

                // Perform FSM transition
                sqlx::query("SELECT lib_fsm.state_machine_transition($1, $2)")
                    .bind(state_machine_id)
                    .bind(event)
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(|e| StoreError::Database(e.to_string()))?;
            }

            let now = Utc::now();
            let mut sets = vec!["updated_at = $1".to_string()];
            let mut bind_idx = 2u32;

            macro_rules! push_set {
                ($field:expr, $val:expr) => {
                    if $val.is_some() {
                        sets.push(format!("{} = ${}", $field, bind_idx));
                        bind_idx += 1;
                    }
                };
            }

            push_set!("output", update.output);
            push_set!("error", update.error);
            push_set!("duration_ms", update.duration_ms);
            push_set!("cost_usd", update.cost_usd);
            push_set!("input_tokens", update.input_tokens);
            push_set!("output_tokens", update.output_tokens);
            push_set!("started_at", update.started_at);
            push_set!("completed_at", update.completed_at);
            push_set!("debug_messages", update.debug_messages);

            let sql = format!(
                "UPDATE ironflow.steps SET {} WHERE id = ${bind_idx}",
                sets.join(", ")
            );

            let mut query = sqlx::query(&sql).bind(now);

            if let Some(ref output) = update.output {
                query = query.bind(output);
            }
            if let Some(ref error) = update.error {
                query = query.bind(error);
            }
            if let Some(dur) = update.duration_ms {
                query = query.bind(dur as i64);
            }
            if let Some(cost) = update.cost_usd {
                query = query.bind(cost);
            }
            if let Some(tokens) = update.input_tokens {
                query = query.bind(tokens as i64);
            }
            if let Some(tokens) = update.output_tokens {
                query = query.bind(tokens as i64);
            }
            if let Some(started) = update.started_at {
                query = query.bind(started);
            }
            if let Some(completed) = update.completed_at {
                query = query.bind(completed);
            }
            if let Some(ref debug_msgs) = update.debug_messages {
                query = query.bind(debug_msgs);
            }

            query = query.bind(id);

            let result = query
                .execute(&mut *tx)
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            if result.rows_affected() == 0 {
                return Err(StoreError::StepNotFound(id));
            }

            tx.commit()
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(())
        })
    }

    fn get_step(&self, id: Uuid) -> StoreFuture<'_, Option<Step>> {
        Box::pin(async move {
            let row = sqlx::query(
                r#"
                SELECT s.*, ast.name as state_name
                FROM ironflow.steps s
                JOIN lib_fsm.state_machine sm ON sm.state_machine__id = s.state_machine__id
                JOIN lib_fsm.abstract_state ast ON ast.abstract_state__id = sm.abstract_state__id
                WHERE s.id = $1
                "#,
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            row.map(|r| row_to_step(&r)).transpose()
        })
    }

    fn list_steps(&self, run_id: Uuid) -> StoreFuture<'_, Vec<Step>> {
        Box::pin(async move {
            let rows = sqlx::query(
                r#"
                SELECT s.*, ast.name as state_name
                FROM ironflow.steps s
                JOIN lib_fsm.state_machine sm ON sm.state_machine__id = s.state_machine__id
                JOIN lib_fsm.abstract_state ast ON ast.abstract_state__id = sm.abstract_state__id
                WHERE s.run_id = $1
                ORDER BY s.position ASC
                "#,
            )
            .bind(run_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            rows.iter().map(row_to_step).collect()
        })
    }

    fn get_stats(&self, filter: RunFilter) -> StoreFuture<'_, RunStats> {
        Box::pin(async move {
            let (where_clause, _) = build_run_filter_conditions(&filter);

            let sql = format!(
                r#"
                SELECT
                    COUNT(*) as total,
                    COUNT(*) FILTER (WHERE ast.name = 'completed') as completed,
                    COUNT(*) FILTER (WHERE ast.name = 'failed') as failed,
                    COUNT(*) FILTER (WHERE ast.name = 'cancelled') as cancelled,
                    COUNT(*) FILTER (WHERE ast.name IN ('pending', 'running', 'retrying')) as active,
                    COALESCE(SUM(r.cost_usd), 0) as total_cost,
                    COALESCE(SUM(r.duration_ms), 0)::BIGINT as total_duration
                FROM ironflow.runs r
                JOIN lib_fsm.state_machine sm ON sm.state_machine__id = r.state_machine__id
                JOIN lib_fsm.abstract_state ast ON ast.abstract_state__id = sm.abstract_state__id
                {where_clause}
                "#
            );

            let row = bind_run_filter_params(sqlx::query(&sql), &filter)
                .fetch_one(&self.pool)
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(RunStats {
                total_runs: row.get::<i64, _>("total") as u64,
                completed_runs: row.get::<i64, _>("completed") as u64,
                failed_runs: row.get::<i64, _>("failed") as u64,
                cancelled_runs: row.get::<i64, _>("cancelled") as u64,
                active_runs: row.get::<i64, _>("active") as u64,
                total_cost_usd: row.get::<rust_decimal::Decimal, _>("total_cost"),
                total_duration_ms: row.get::<i64, _>("total_duration") as u64,
            })
        })
    }

    fn create_step_dependencies(&self, deps: Vec<NewStepDependency>) -> StoreFuture<'_, ()> {
        Box::pin(async move {
            if deps.is_empty() {
                return Ok(());
            }

            let mut query = String::from(
                "INSERT INTO ironflow.step_dependencies (step_id, depends_on) VALUES ",
            );
            let mut binds: Vec<Uuid> = Vec::with_capacity(deps.len() * 2);

            for (i, dep) in deps.iter().enumerate() {
                if i > 0 {
                    query.push_str(", ");
                }
                let p1 = i * 2 + 1;
                let p2 = i * 2 + 2;
                query.push_str(&format!("(${p1}, ${p2})"));
                binds.push(dep.step_id);
                binds.push(dep.depends_on);
            }

            query.push_str(" ON CONFLICT DO NOTHING");

            let mut q = sqlx::query(&query);
            for id in &binds {
                q = q.bind(id);
            }

            q.execute(&self.pool)
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(())
        })
    }

    fn list_step_dependencies(&self, run_id: Uuid) -> StoreFuture<'_, Vec<StepDependency>> {
        Box::pin(async move {
            let rows = sqlx::query(
                r#"
                SELECT sd.step_id, sd.depends_on, sd.created_at
                FROM ironflow.step_dependencies sd
                JOIN ironflow.steps s ON s.id = sd.step_id
                WHERE s.run_id = $1
                ORDER BY sd.created_at ASC
                "#,
            )
            .bind(run_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(rows
                .iter()
                .map(|row| StepDependency {
                    step_id: row.get("step_id"),
                    depends_on: row.get("depends_on"),
                    created_at: row.get("created_at"),
                })
                .collect())
        })
    }
}
