//! PostgreSQL [`RunStore`] implementation for production use.
//!
//! Uses [`sqlx`] with connection pooling and `SELECT FOR UPDATE SKIP LOCKED`
//! for safe multi-worker concurrency.
//!
//! Integrates with the `lib_fsm` schema for SQL-side state machine management.
//!
//! # Examples
//!
//! ```no_run
//! use ironflow_store::postgres::PostgresStore;
//!
//! # async fn example() -> Result<(), ironflow_store::error::StoreError> {
//! let store = PostgresStore::new("postgres://localhost/ironflow").await?;
//! // Use store as Arc<dyn RunStore>
//! # Ok(())
//! # }
//! ```

mod helpers;

use chrono::Utc;
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};
use tracing::info;
use uuid::Uuid;

use crate::entities::{
    NewRun, NewStep, NewStepDependency, NewUser, Page, Run, RunFilter, RunStats, RunStatus,
    RunUpdate, Step, StepDependency, StepUpdate, User,
};
use crate::error::StoreError;
use crate::store::{RunStore, StoreFuture};
use crate::user_store::UserStore;

use helpers::row_to_run;
use helpers::row_to_step;

/// PostgreSQL-backed run store with SQL-side FSM support.
///
/// Manages a connection pool internally. Clone is cheap (Arc-based pool).
///
/// # Examples
///
/// ```no_run
/// use ironflow_store::postgres::PostgresStore;
///
/// # async fn example() -> Result<(), ironflow_store::error::StoreError> {
/// let store = PostgresStore::new("postgres://user:pass@localhost/ironflow").await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct PostgresStore {
    pool: PgPool,
    run_lifecycle_id: Uuid,
    step_lifecycle_id: Uuid,
}

impl PostgresStore {
    /// Connect to PostgreSQL and run migrations.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] if the connection or migration fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_store::postgres::PostgresStore;
    ///
    /// # async fn example() -> Result<(), ironflow_store::error::StoreError> {
    /// let store = PostgresStore::new("postgres://localhost/ironflow").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn new(database_url: &str) -> Result<Self, StoreError> {
        let pool = PgPoolOptions::new()
            .max_connections(20)
            .connect(database_url)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .map_err(|e| StoreError::Database(format!("migration failed: {e}")))?;

        let run_lifecycle_id = Self::fetch_machine_id(&pool, "run_lifecycle").await?;
        let step_lifecycle_id = Self::fetch_machine_id(&pool, "step_lifecycle").await?;

        info!("PostgresStore connected and migrations applied");
        Ok(Self {
            pool,
            run_lifecycle_id,
            step_lifecycle_id,
        })
    }

    /// Create from an existing connection pool (skips migrations).
    ///
    /// Useful for tests where migrations are managed externally.
    pub async fn from_pool(pool: PgPool) -> Result<Self, StoreError> {
        let run_lifecycle_id = Self::fetch_machine_id(&pool, "run_lifecycle").await?;
        let step_lifecycle_id = Self::fetch_machine_id(&pool, "step_lifecycle").await?;

        Ok(Self {
            pool,
            run_lifecycle_id,
            step_lifecycle_id,
        })
    }

    /// Fetch a machine ID by name from the database.
    async fn fetch_machine_id(pool: &PgPool, name: &str) -> Result<Uuid, StoreError> {
        let row = sqlx::query(
            "SELECT abstract_machine__id FROM lib_fsm.abstract_state_machine WHERE name = $1",
        )
        .bind(name)
        .fetch_one(pool)
        .await
        .map_err(|e| StoreError::Database(format!("failed to get {name} FSM: {e}")))?;

        Ok(row.get("abstract_machine__id"))
    }

    /// Get the cached abstract machine ID for run_lifecycle FSM.
    fn get_run_lifecycle_machine_id(&self) -> Uuid {
        self.run_lifecycle_id
    }

    /// Get the cached abstract machine ID for step_lifecycle FSM.
    fn get_step_lifecycle_machine_id(&self) -> Uuid {
        self.step_lifecycle_id
    }

    /// Map RunStatus to FSM event name.
    fn run_status_to_event(from: RunStatus, to: RunStatus) -> Result<&'static str, StoreError> {
        match (from, to) {
            (RunStatus::Pending, RunStatus::Running) => Ok("picked_up"),
            (RunStatus::Pending, RunStatus::Cancelled) => Ok("cancel_requested"),
            (RunStatus::Running, RunStatus::Completed) => Ok("all_steps_completed"),
            (RunStatus::Running, RunStatus::Failed) => Ok("step_failed"),
            (RunStatus::Running, RunStatus::Retrying) => Ok("step_failed_retryable"),
            (RunStatus::Running, RunStatus::Cancelled) => Ok("cancel_requested"),
            (RunStatus::Retrying, RunStatus::Running) => Ok("retry_started"),
            (RunStatus::Retrying, RunStatus::Failed) => Ok("max_retries_exceeded"),
            (RunStatus::Retrying, RunStatus::Cancelled) => Ok("cancel_requested"),
            (RunStatus::Running, RunStatus::AwaitingApproval) => Ok("approval_requested"),
            (RunStatus::AwaitingApproval, RunStatus::Running) => Ok("approved"),
            (RunStatus::AwaitingApproval, RunStatus::Failed) => Ok("rejected"),
            (RunStatus::AwaitingApproval, RunStatus::Cancelled) => Ok("cancel_requested"),
            _ => Err(StoreError::InvalidTransition { from, to }),
        }
    }

    /// Map StepStatus to FSM event name.
    fn step_status_to_event(
        from: crate::entities::StepStatus,
        to: crate::entities::StepStatus,
    ) -> Result<&'static str, StoreError> {
        use crate::entities::StepStatus;
        match (from, to) {
            (StepStatus::Pending, StepStatus::Running) => Ok("started"),
            (StepStatus::Pending, StepStatus::Skipped) => Ok("skipped"),
            (StepStatus::Running, StepStatus::Completed) => Ok("succeeded"),
            (StepStatus::Running, StepStatus::Failed) => Ok("failed"),
            _ => Err(StoreError::Database(format!(
                "invalid step status transition: {from:?} -> {to:?}"
            ))),
        }
    }
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
            let sm_row =
                sqlx::query("SELECT lib_fsm.state_machine_create($1) as state_machine__id")
                    .bind(fsm_machine_id)
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(|e| {
                        StoreError::Database(format!("failed to create FSM instance: {e}"))
                    })?;

            let state_machine_id: Uuid = sm_row.get("state_machine__id");

            // Insert run with FSM reference
            sqlx::query(
                r#"
                INSERT INTO ironflow.runs (id, workflow_name, state_machine__id, trigger, payload, max_retries, created_at, updated_at)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                "#,
            )
            .bind(id)
            .bind(&req.workflow_name)
            .bind(state_machine_id)
            .bind(&trigger_json)
            .bind(&req.payload)
            .bind(req.max_retries as i32)
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

            let where_clause = if conditions.is_empty() {
                String::new()
            } else {
                format!("WHERE {}", conditions.join(" AND "))
            };

            // Use window function to get total count without a separate query
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
            let mut data_query = sqlx::query(&data_sql);

            if let Some(ref wf) = filter.workflow_name {
                data_query = data_query.bind(format!("%{wf}%"));
            }
            if let Some(ref status) = filter.status {
                data_query = data_query.bind(helpers::run_status_to_db_str(status));
            }
            if let Some(after) = filter.created_after {
                data_query = data_query.bind(after);
            }
            if let Some(before) = filter.created_before {
                data_query = data_query.bind(before);
            }

            data_query = data_query.bind(per_page as i64).bind(offset);

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
            let current = helpers::parse_run_status(current_state_name)?;

            if !current.can_transition_to(&new_status) {
                return Err(StoreError::InvalidTransition {
                    from: current,
                    to: new_status,
                });
            }

            let event = Self::run_status_to_event(current, new_status)?;
            let now = Utc::now();

            let state_machine_id: Uuid = row.get("state_machine__id");

            // Perform FSM transition
            sqlx::query("SELECT lib_fsm.state_machine_transition($1, $2)")
                .bind(state_machine_id)
                .bind(event)
                .execute(&mut *tx)
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

            // Handle status transition if provided
            if let Some(new_status) = update.status {
                let row = sqlx::query(
                    r#"
                    SELECT ast.name as state_name, r.state_machine__id
                    FROM ironflow.runs r
                    JOIN lib_fsm.state_machine sm ON sm.state_machine__id = r.state_machine__id
                    JOIN lib_fsm.abstract_state ast ON ast.abstract_state__id = sm.abstract_state__id
                    WHERE r.id = $1
                    FOR UPDATE
                    "#
                )
                .bind(id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?
                .ok_or(StoreError::RunNotFound(id))?;

                let current_state_name: &str = row.get("state_name");
                let current = helpers::parse_run_status(current_state_name)?;

                if !current.can_transition_to(&new_status) {
                    return Err(StoreError::InvalidTransition {
                        from: current,
                        to: new_status,
                    });
                }

                let event = Self::run_status_to_event(current, new_status)?;
                let state_machine_id: Uuid = row.get("state_machine__id");

                // Perform FSM transition
                sqlx::query("SELECT lib_fsm.state_machine_transition($1, $2)")
                    .bind(state_machine_id)
                    .bind(event)
                    .execute(&mut *tx)
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

            push_set!("error", update.error);
            push_set!("cost_usd", update.cost_usd);
            push_set!("duration_ms", update.duration_ms);
            push_set!("started_at", update.started_at);
            push_set!("completed_at", update.completed_at);

            if update.increment_retry {
                sets.push("retry_count = retry_count + 1".to_string());
            }

            let sql = format!(
                "UPDATE ironflow.runs SET {} WHERE id = ${bind_idx}",
                sets.join(", ")
            );

            let mut query = sqlx::query(&sql).bind(now);

            if let Some(ref error) = update.error {
                query = query.bind(error);
            }
            if let Some(cost) = update.cost_usd {
                query = query.bind(cost);
            }
            if let Some(dur) = update.duration_ms {
                query = query.bind(dur as i64);
            }
            if let Some(started) = update.started_at {
                query = query.bind(started);
            }
            if let Some(completed) = update.completed_at {
                query = query.bind(completed);
            }

            query = query.bind(id);

            let result = query
                .execute(&mut *tx)
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            if result.rows_affected() == 0 {
                return Err(StoreError::RunNotFound(id));
            }

            tx.commit()
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(())
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
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| StoreError::Database(e.to_string()))?;

                // Update timestamps
                sqlx::query(
                    "UPDATE ironflow.runs SET started_at = COALESCE(started_at, $1), updated_at = $1 WHERE id = $2"
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
            let sm_row =
                sqlx::query("SELECT lib_fsm.state_machine_create($1) as state_machine__id")
                    .bind(fsm_machine_id)
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(|e| {
                        StoreError::Database(format!("failed to create FSM instance: {e}"))
                    })?;

            let state_machine_id: Uuid = sm_row.get("state_machine__id");

            sqlx::query(
                r#"
                INSERT INTO ironflow.steps (id, run_id, name, kind, position, state_machine__id, input, created_at, updated_at)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                "#,
            )
            .bind(id)
            .bind(req.run_id)
            .bind(&req.name)
            .bind(helpers::step_kind_to_str(&req.kind).as_ref())
            .bind(req.position as i32)
            .bind(state_machine_id)
            .bind(&req.input)
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
                let current = helpers::parse_step_status(current_state_name)?;

                let event = Self::step_status_to_event(current, new_status).map_err(|_| {
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
                    .execute(&mut *tx)
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

    fn get_stats(&self) -> StoreFuture<'_, RunStats> {
        Box::pin(async move {
            let row = sqlx::query(
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
                "#
            )
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

impl UserStore for PostgresStore {
    fn create_user(&self, req: NewUser) -> StoreFuture<'_, User> {
        Box::pin(async move {
            let id = Uuid::now_v7();
            let now = Utc::now();

            let row = sqlx::query(
                r#"
                INSERT INTO iam.users (id, email, username, password_hash, is_admin, created_at, updated_at)
                VALUES ($1, $2, $3, $4, FALSE, $5, $6)
                RETURNING *
                "#,
            )
            .bind(id)
            .bind(&req.email)
            .bind(&req.username)
            .bind(&req.password_hash)
            .bind(now)
            .bind(now)
            .fetch_one(&self.pool)
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
            let row = sqlx::query("SELECT * FROM iam.users WHERE email = $1")
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
            let row = sqlx::query("SELECT * FROM iam.users WHERE id = $1")
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
}
