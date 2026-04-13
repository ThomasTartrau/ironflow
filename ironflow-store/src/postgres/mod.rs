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

use std::time::Duration;

use chrono::Utc;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{PgPool, Row};
use tracing::info;
use uuid::Uuid;

use crate::api_key_store::ApiKeyStore;
use crate::entities::{
    ApiKey, ApiKeyScope, ApiKeyUpdate, NewApiKey, NewRun, NewStep, NewStepDependency, NewUser,
    Page, Run, RunFilter, RunStats, RunStatus, RunUpdate, Step, StepDependency, StepUpdate, User,
};
use crate::error::StoreError;
use crate::store::{RunStore, StoreFuture};
use crate::user_store::UserStore;

use helpers::row_to_run;
use helpers::row_to_step;

/// Configuration for the PostgreSQL connection pool.
///
/// Controls pool sizing, connection timeouts, and acquisition timeouts.
/// All fields have sensible defaults for production use.
///
/// # Examples
///
/// ```
/// use ironflow_store::postgres::PoolConfig;
/// use std::time::Duration;
///
/// // Use defaults
/// let config = PoolConfig::default();
/// assert_eq!(config.max_connections, 20);
///
/// // Customize for high-throughput
/// let config = PoolConfig {
///     max_connections: 50,
///     min_connections: 5,
///     connect_timeout: Duration::from_secs(10),
///     acquire_timeout: Duration::from_secs(10),
///     idle_timeout: Some(Duration::from_secs(600)),
///     max_lifetime: Some(Duration::from_secs(1800)),
/// };
/// ```
#[derive(Debug, Clone)]
pub struct PoolConfig {
    /// Maximum number of connections in the pool.
    pub max_connections: u32,
    /// Minimum number of idle connections to maintain.
    pub min_connections: u32,
    /// Timeout for establishing a new connection.
    pub connect_timeout: Duration,
    /// Timeout for acquiring a connection from the pool.
    pub acquire_timeout: Duration,
    /// Maximum idle duration before a connection is closed. `None` means no limit.
    pub idle_timeout: Option<Duration>,
    /// Maximum lifetime of a connection. `None` means no limit.
    pub max_lifetime: Option<Duration>,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            max_connections: 20,
            min_connections: 1,
            connect_timeout: Duration::from_secs(5),
            acquire_timeout: Duration::from_secs(5),
            idle_timeout: Some(Duration::from_secs(600)),
            max_lifetime: Some(Duration::from_secs(1800)),
        }
    }
}

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
    /// Connect to PostgreSQL with default [`PoolConfig`] and run migrations.
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
        Self::with_config(database_url, PoolConfig::default()).await
    }

    /// Connect to PostgreSQL with custom [`PoolConfig`] and run migrations.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] if the connection or migration fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_store::postgres::{PostgresStore, PoolConfig};
    /// use std::time::Duration;
    ///
    /// # async fn example() -> Result<(), ironflow_store::error::StoreError> {
    /// let config = PoolConfig {
    ///     max_connections: 50,
    ///     connect_timeout: Duration::from_secs(10),
    ///     ..PoolConfig::default()
    /// };
    /// let store = PostgresStore::with_config("postgres://localhost/ironflow", config).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn with_config(database_url: &str, config: PoolConfig) -> Result<Self, StoreError> {
        let connect_options = database_url
            .parse::<PgConnectOptions>()
            .map_err(|e| StoreError::Database(e.to_string()))?;

        let mut pool_options = PgPoolOptions::new()
            .max_connections(config.max_connections)
            .min_connections(config.min_connections)
            .acquire_timeout(config.acquire_timeout);

        if let Some(idle_timeout) = config.idle_timeout {
            pool_options = pool_options.idle_timeout(idle_timeout);
        }
        if let Some(max_lifetime) = config.max_lifetime {
            pool_options = pool_options.max_lifetime(max_lifetime);
        }

        let pool = pool_options
            .connect_with(connect_options)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

        // Verify connection is actually usable (not just pooled)
        sqlx::query!("SELECT 1 as ok")
            .fetch_one(&pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .map_err(|e| StoreError::Database(format!("migration failed: {e}")))?;

        let run_lifecycle_id = Self::fetch_machine_id(&pool, "run_lifecycle").await?;
        let step_lifecycle_id = Self::fetch_machine_id(&pool, "step_lifecycle").await?;

        info!(
            max_connections = config.max_connections,
            acquire_timeout_secs = config.acquire_timeout.as_secs(),
            "PostgresStore connected and migrations applied"
        );
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

    /// Check that the database connection is alive.
    ///
    /// Useful for readiness probes (e.g. Kubernetes `/ready` endpoint).
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] if the connection is unhealthy.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_store::postgres::PostgresStore;
    ///
    /// # async fn example() -> Result<(), ironflow_store::error::StoreError> {
    /// let store = PostgresStore::new("postgres://localhost/ironflow").await?;
    /// store.test_connection().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn test_connection(&self) -> Result<(), StoreError> {
        sqlx::query!("SELECT 1 as ok")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;
        Ok(())
    }

    /// Fetch a machine ID by name from the database.
    async fn fetch_machine_id(pool: &PgPool, name: &str) -> Result<Uuid, StoreError> {
        let row = sqlx::query!(
            "SELECT abstract_machine__id FROM lib_fsm.abstract_state_machine WHERE name = $1",
            name,
        )
        .fetch_one(pool)
        .await
        .map_err(|e| StoreError::Database(format!("failed to get {name} FSM: {e}")))?;

        Ok(row.abstract_machine__id)
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

    /// Apply a partial update to a run within an existing transaction.
    ///
    /// Handles FSM status transition and dynamic SQL UPDATE building.
    /// Callers are responsible for committing the transaction.
    async fn apply_run_update(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        id: Uuid,
        update: &RunUpdate,
    ) -> Result<(), StoreError> {
        if let Some(new_status) = update.status {
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
            .fetch_optional(&mut **tx)
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

            sqlx::query!(
                "SELECT lib_fsm.state_machine_transition($1, $2)",
                state_machine_id,
                event,
            )
            .fetch_one(&mut **tx)
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
            .execute(&mut **tx)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(StoreError::RunNotFound(id));
        }

        Ok(())
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
            (StepStatus::Running, StepStatus::AwaitingApproval) => Ok("suspended"),
            (StepStatus::AwaitingApproval, StepStatus::Running) => Ok("resumed"),
            (StepStatus::AwaitingApproval, StepStatus::Rejected) => Ok("rejected"),
            (StepStatus::AwaitingApproval, StepStatus::Failed) => Ok("failed"),
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
            let state_machine_id: Uuid = sqlx::query_scalar!(
                "SELECT lib_fsm.state_machine_create($1) as \"state_machine__id!\"",
                fsm_machine_id,
            )
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| StoreError::Database(format!("failed to create FSM instance: {e}")))?;

            // Insert run with FSM reference
            sqlx::query!(
                r#"
                INSERT INTO ironflow.runs (id, workflow_name, state_machine__id, trigger, payload, max_retries, created_at, updated_at)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                "#,
                id,
                &req.workflow_name,
                state_machine_id,
                &trigger_json,
                &req.payload,
                req.max_retries as i32,
                now,
                now,
            )
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
            sqlx::query!(
                "SELECT lib_fsm.state_machine_transition($1, $2)",
                state_machine_id,
                event,
            )
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

            Self::apply_run_update(&mut tx, id, &update).await?;

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

            Self::apply_run_update(&mut tx, id, &update).await?;

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
                sqlx::query!(
                    "SELECT lib_fsm.state_machine_transition($1, $2)",
                    state_machine_id,
                    event,
                )
                .fetch_one(&mut *tx)
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

                // Update timestamps
                sqlx::query!(
                    "UPDATE ironflow.runs SET started_at = COALESCE(started_at, $1), updated_at = $1 WHERE id = $2",
                    now,
                    run_id,
                )
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
            let state_machine_id: Uuid = sqlx::query_scalar!(
                "SELECT lib_fsm.state_machine_create($1) as \"state_machine__id!\"",
                fsm_machine_id,
            )
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| StoreError::Database(format!("failed to create FSM instance: {e}")))?;

            let kind_str = helpers::step_kind_to_str(&req.kind);
            sqlx::query!(
                r#"
                INSERT INTO ironflow.steps (id, run_id, name, kind, position, state_machine__id, input, created_at, updated_at)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                "#,
                id,
                req.run_id,
                &req.name,
                kind_str.as_ref(),
                req.position as i32,
                state_machine_id,
                req.input.as_ref(),
                now,
                now,
            )
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
                sqlx::query!(
                    "SELECT lib_fsm.state_machine_transition($1, $2)",
                    state_machine_id,
                    event,
                )
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

            let row = sqlx::query!(
                r#"
                INSERT INTO iam.users (id, email, username, password_hash, is_admin, created_at, updated_at)
                VALUES ($1, $2, $3, $4, FALSE, $5, $6)
                RETURNING id, email, username, password_hash, is_admin, created_at, updated_at
                "#,
                id,
                &req.email,
                &req.username,
                &req.password_hash,
                now,
                now,
            )
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
                id: row.id,
                email: row.email,
                username: row.username,
                password_hash: row.password_hash,
                is_admin: row.is_admin,
                created_at: row.created_at,
                updated_at: row.updated_at,
            })
        })
    }

    fn find_user_by_email(&self, email: &str) -> StoreFuture<'_, Option<User>> {
        let email = email.to_string();
        Box::pin(async move {
            let row = sqlx::query!(
                "SELECT id, email, username, password_hash, is_admin, created_at, updated_at FROM iam.users WHERE email = $1",
                &email,
            )
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(row.map(|r| User {
                id: r.id,
                email: r.email,
                username: r.username,
                password_hash: r.password_hash,
                is_admin: r.is_admin,
                created_at: r.created_at,
                updated_at: r.updated_at,
            }))
        })
    }

    fn find_user_by_id(&self, id: Uuid) -> StoreFuture<'_, Option<User>> {
        Box::pin(async move {
            let row = sqlx::query!(
                "SELECT id, email, username, password_hash, is_admin, created_at, updated_at FROM iam.users WHERE id = $1",
                id,
            )
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(row.map(|r| User {
                id: r.id,
                email: r.email,
                username: r.username,
                password_hash: r.password_hash,
                is_admin: r.is_admin,
                created_at: r.created_at,
                updated_at: r.updated_at,
            }))
        })
    }
}

fn parse_scopes(raw: Vec<String>) -> Result<Vec<ApiKeyScope>, StoreError> {
    raw.into_iter()
        .map(|s| {
            s.parse::<ApiKeyScope>()
                .map_err(|e| StoreError::Database(e.to_string()))
        })
        .collect()
}

fn scopes_to_strings(scopes: &[ApiKeyScope]) -> Vec<String> {
    scopes.iter().map(|s| s.to_string()).collect()
}

impl ApiKeyStore for PostgresStore {
    fn create_api_key(&self, req: NewApiKey) -> StoreFuture<'_, ApiKey> {
        Box::pin(async move {
            let id = Uuid::now_v7();
            let now = Utc::now();
            let scopes_str = scopes_to_strings(&req.scopes);

            let row = sqlx::query!(
                r#"
                INSERT INTO iam.api_keys (id, user_id, name, key_hash, key_prefix, scopes, is_active, expires_at, created_at, updated_at)
                VALUES ($1, $2, $3, $4, $5, $6, TRUE, $7, $8, $9)
                RETURNING id, user_id, name, key_hash, key_prefix, scopes, is_active, expires_at, last_used_at, created_at, updated_at
                "#,
                id,
                req.user_id,
                &req.name,
                &req.key_hash,
                &req.key_prefix,
                &scopes_str,
                req.expires_at,
                now,
                now,
            )
            .fetch_one(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(ApiKey {
                id: row.id,
                user_id: row.user_id,
                name: row.name,
                key_hash: row.key_hash,
                key_prefix: row.key_prefix,
                scopes: parse_scopes(row.scopes)?,
                is_active: row.is_active,
                expires_at: row.expires_at,
                last_used_at: row.last_used_at,
                created_at: row.created_at,
                updated_at: row.updated_at,
            })
        })
    }

    fn find_api_key_by_prefix(&self, prefix: &str) -> StoreFuture<'_, Option<ApiKey>> {
        let prefix = prefix.to_string();
        Box::pin(async move {
            let row = sqlx::query!(
                r#"
                SELECT id, user_id, name, key_hash, key_prefix, scopes, is_active, expires_at, last_used_at, created_at, updated_at
                FROM iam.api_keys WHERE key_prefix = $1 AND is_active = TRUE
                "#,
                &prefix,
            )
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            row.map(|r| {
                Ok(ApiKey {
                    id: r.id,
                    user_id: r.user_id,
                    name: r.name,
                    key_hash: r.key_hash,
                    key_prefix: r.key_prefix,
                    scopes: parse_scopes(r.scopes)?,
                    is_active: r.is_active,
                    expires_at: r.expires_at,
                    last_used_at: r.last_used_at,
                    created_at: r.created_at,
                    updated_at: r.updated_at,
                })
            })
            .transpose()
        })
    }

    fn find_api_key_by_id(&self, id: Uuid) -> StoreFuture<'_, Option<ApiKey>> {
        Box::pin(async move {
            let row = sqlx::query!(
                r#"
                SELECT id, user_id, name, key_hash, key_prefix, scopes, is_active, expires_at, last_used_at, created_at, updated_at
                FROM iam.api_keys WHERE id = $1
                "#,
                id,
            )
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            row.map(|r| {
                Ok(ApiKey {
                    id: r.id,
                    user_id: r.user_id,
                    name: r.name,
                    key_hash: r.key_hash,
                    key_prefix: r.key_prefix,
                    scopes: parse_scopes(r.scopes)?,
                    is_active: r.is_active,
                    expires_at: r.expires_at,
                    last_used_at: r.last_used_at,
                    created_at: r.created_at,
                    updated_at: r.updated_at,
                })
            })
            .transpose()
        })
    }

    fn list_api_keys_by_user(&self, user_id: Uuid) -> StoreFuture<'_, Vec<ApiKey>> {
        Box::pin(async move {
            let rows = sqlx::query!(
                r#"
                SELECT id, user_id, name, key_hash, key_prefix, scopes, is_active, expires_at, last_used_at, created_at, updated_at
                FROM iam.api_keys WHERE user_id = $1 ORDER BY created_at DESC
                "#,
                user_id,
            )
            .fetch_all(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            rows.into_iter()
                .map(|r| {
                    Ok(ApiKey {
                        id: r.id,
                        user_id: r.user_id,
                        name: r.name,
                        key_hash: r.key_hash,
                        key_prefix: r.key_prefix,
                        scopes: parse_scopes(r.scopes)?,
                        is_active: r.is_active,
                        expires_at: r.expires_at,
                        last_used_at: r.last_used_at,
                        created_at: r.created_at,
                        updated_at: r.updated_at,
                    })
                })
                .collect()
        })
    }

    fn update_api_key(&self, id: Uuid, update: ApiKeyUpdate) -> StoreFuture<'_, ()> {
        Box::pin(async move {
            let now = Utc::now();
            let scopes_str: Option<Vec<String>> =
                update.scopes.map(|scopes| scopes_to_strings(&scopes));
            let has_expires_at = update.expires_at.is_some();
            let expires_at_value = update.expires_at.flatten();

            sqlx::query!(
                r#"
                UPDATE iam.api_keys
                SET name       = COALESCE($2, name),
                    scopes     = COALESCE($3, scopes),
                    is_active  = COALESCE($4, is_active),
                    expires_at = CASE WHEN $5 THEN $6 ELSE expires_at END,
                    updated_at = $7
                WHERE id = $1
                "#,
                id,
                update.name,
                scopes_str.as_deref(),
                update.is_active,
                has_expires_at,
                expires_at_value,
                now,
            )
            .execute(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(())
        })
    }

    fn touch_api_key(&self, id: Uuid) -> StoreFuture<'_, ()> {
        Box::pin(async move {
            sqlx::query!(
                "UPDATE iam.api_keys SET last_used_at = NOW() WHERE id = $1",
                id,
            )
            .execute(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(())
        })
    }

    fn delete_api_key(&self, id: Uuid) -> StoreFuture<'_, ()> {
        Box::pin(async move {
            sqlx::query!("DELETE FROM iam.api_keys WHERE id = $1", id)
                .execute(&self.pool)
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;

            Ok(())
        })
    }
}
