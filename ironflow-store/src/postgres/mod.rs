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

mod api_key_store;
mod artifact_store;
mod audit_log_store;
mod helpers;
mod run_store;
mod secret_store;
mod user_store;

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{PgPool, Row};
use tracing::info;
use uuid::Uuid;

use crate::entities::{RunStatus, RunUpdate};
use crate::error::StoreError;

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
    pub(super) pool: PgPool,
    pub(super) run_lifecycle_id: Uuid,
    pub(super) step_lifecycle_id: Uuid,
    #[cfg(feature = "secret-store")]
    pub(super) key_ring: Option<Arc<crate::crypto::KeyRing>>,
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
        sqlx::query("SELECT 1 as ok")
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
            #[cfg(feature = "secret-store")]
            key_ring: None,
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
            #[cfg(feature = "secret-store")]
            key_ring: None,
        })
    }

    /// Set a single, unversioned master key for secret encryption.
    ///
    /// Shorthand for a key ring holding this key alone at
    /// [`LEGACY_KEY_VERSION`](crate::crypto::LEGACY_KEY_VERSION), which is how
    /// the deprecated `IRONFLOW_SECRET_KEY` variable is interpreted.
    ///
    /// Required before using [`SecretStore`](crate::secret_store::SecretStore)
    /// methods. Call this after constructing the store.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::env::var;
    ///
    /// use ironflow_store::postgres::PostgresStore;
    /// use ironflow_store::crypto::MasterKey;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut store = PostgresStore::new("postgres://localhost/ironflow").await?;
    /// let key = MasterKey::from_hex(&var("IRONFLOW_SECRET_KEY")?)?;
    /// store.set_master_key(key);
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(feature = "secret-store")]
    pub fn set_master_key(&mut self, key: crate::crypto::MasterKey) {
        self.set_key_ring(crate::crypto::KeyRing::single(key));
    }

    /// Set the versioned key ring for secret encryption.
    ///
    /// New secrets are encrypted with the ring's active version; existing ones
    /// are decrypted with whichever version they were written with. This is
    /// what lets [`SecretStore::rotate_secrets`](crate::secret_store::SecretStore::rotate_secrets)
    /// re-encrypt the stock while the service keeps serving.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::env::var;
    ///
    /// use ironflow_store::postgres::PostgresStore;
    /// use ironflow_store::crypto::KeyRing;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut store = PostgresStore::new("postgres://localhost/ironflow").await?;
    /// let active = var("IRONFLOW_SECRET_ACTIVE_KEY_VERSION").ok()
    ///     .map(|v| v.parse::<i32>())
    ///     .transpose()?;
    /// store.set_key_ring(KeyRing::from_spec(&var("IRONFLOW_SECRET_KEYS")?, active)?);
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(feature = "secret-store")]
    pub fn set_key_ring(&mut self, ring: crate::crypto::KeyRing) {
        self.key_ring = Some(Arc::new(ring));
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
        sqlx::query("SELECT 1 as ok")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;
        Ok(())
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
    pub(super) fn get_run_lifecycle_machine_id(&self) -> Uuid {
        self.run_lifecycle_id
    }

    /// Get the cached abstract machine ID for step_lifecycle FSM.
    pub(super) fn get_step_lifecycle_machine_id(&self) -> Uuid {
        self.step_lifecycle_id
    }

    /// Map RunStatus to FSM event name.
    pub(super) fn run_status_to_event(
        from: RunStatus,
        to: RunStatus,
    ) -> Result<&'static str, StoreError> {
        match (from, to) {
            (RunStatus::Pending, RunStatus::Running) => Ok("picked_up"),
            (RunStatus::Running, RunStatus::Pending) => Ok("lease_expired"),
            (RunStatus::Pending, RunStatus::Cancelled) => Ok("cancel_requested"),
            (RunStatus::Running, RunStatus::Completed) => Ok("all_steps_completed"),
            (RunStatus::Running, RunStatus::Warning) => Ok("completed_with_warnings"),
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
    pub(super) async fn apply_run_update(
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

            if !(current == new_status && new_status.is_terminal()) {
                let event = Self::run_status_to_event(current, new_status)?;
                let state_machine_id: Uuid = row.get("state_machine__id");

                sqlx::query("SELECT lib_fsm.state_machine_transition($1, $2)")
                    .bind(state_machine_id)
                    .bind(event)
                    .fetch_one(&mut **tx)
                    .await
                    .map_err(|e| StoreError::Database(e.to_string()))?;
            }
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
        push_set!("scheduled_at", update.scheduled_at);

        if update.increment_retry {
            sets.push("retry_count = retry_count + 1".to_string());
        }

        // A run that is no longer executing must not keep a worker lease,
        // otherwise the reaper would see a stale expiry on a finished run.
        if update.status.is_some_and(|s| s != RunStatus::Running) {
            sets.push("worker_id = NULL".to_string());
            sets.push("lease_expires_at = NULL".to_string());
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
        if let Some(scheduled) = update.scheduled_at {
            query = query.bind(scheduled);
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
    pub(super) fn step_status_to_event(
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
