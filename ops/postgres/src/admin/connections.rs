//! Connection and query monitoring operations.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgPool, Row};

use crate::helpers::{pg_error, to_value};

/// Output of [`ActiveConnections`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveConnectionsOutput {
    /// Number of active connections.
    pub count: i64,
}

/// Count the number of active connections to the current database.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_postgres::admin::connections::ActiveConnections;
/// use ironflow_core::operation::Operation;
///
/// # fn example(pool: sqlx::PgPool) {
/// let op = ActiveConnections::new(pool);
/// assert_eq!(op.kind(), "postgres");
/// # }
/// ```
pub struct ActiveConnections {
    pool: PgPool,
}

impl ActiveConnections {
    /// Create a new active-connections operation.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] on connection errors.
    pub async fn run(
        &self,
        _ctx: &OperationContext,
    ) -> Result<ActiveConnectionsOutput, OperationError> {
        let row = sqlx::query(
            "SELECT count(*) AS cnt FROM pg_stat_activity \
             WHERE datname = current_database()",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(pg_error)?;
        let count: i64 = row.try_get("cnt").map_err(pg_error)?;
        Ok(ActiveConnectionsOutput { count })
    }
}

#[async_trait]
impl Operation for ActiveConnections {
    fn kind(&self) -> &str {
        "postgres"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
}

impl TypedOperation for ActiveConnections {
    type Output = ActiveConnectionsOutput;
}

/// A running query description.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunningQueryInfo {
    /// Backend process ID.
    pub pid: i32,
    /// The SQL query text.
    pub query: String,
    /// Current state (e.g. `active`, `idle`).
    pub state: String,
    /// How long the query has been running, as a human-readable string.
    pub duration: String,
}

/// Output of [`RunningQueries`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunningQueriesOutput {
    /// Currently running queries.
    pub queries: Vec<RunningQueryInfo>,
}

/// List currently running queries on the database.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_postgres::admin::connections::RunningQueries;
/// use ironflow_core::operation::Operation;
///
/// # fn example(pool: sqlx::PgPool) {
/// let op = RunningQueries::new(pool);
/// assert_eq!(op.kind(), "postgres");
/// # }
/// ```
pub struct RunningQueries {
    pool: PgPool,
}

impl RunningQueries {
    /// Create a new running-queries operation.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] on connection errors.
    pub async fn run(
        &self,
        _ctx: &OperationContext,
    ) -> Result<RunningQueriesOutput, OperationError> {
        let rows = sqlx::query(
            "SELECT pid, query, state, \
                    extract(epoch from (now() - query_start))::bigint AS duration_secs \
             FROM pg_stat_activity \
             WHERE datname = current_database() AND state = 'active' \
             ORDER BY query_start",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(pg_error)?;
        let queries = rows
            .iter()
            .map(|r| {
                let secs: i64 = r.try_get("duration_secs").unwrap_or(0);
                Ok(RunningQueryInfo {
                    pid: r.try_get::<i32, _>("pid").map_err(pg_error)?,
                    query: r.try_get::<String, _>("query").map_err(pg_error)?,
                    state: r.try_get::<String, _>("state").map_err(pg_error)?,
                    duration: format!("{secs}s"),
                })
            })
            .collect::<Result<Vec<_>, OperationError>>()?;
        Ok(RunningQueriesOutput { queries })
    }
}

#[async_trait]
impl Operation for RunningQueries {
    fn kind(&self) -> &str {
        "postgres"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
}

impl TypedOperation for RunningQueries {
    type Output = RunningQueriesOutput;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn active_connections_kind() {
        let pool = PgPool::connect_lazy("postgres://localhost/test").unwrap();
        let op = ActiveConnections::new(pool);
        assert_eq!(op.kind(), "postgres");
    }

    #[tokio::test]
    async fn running_queries_kind() {
        let pool = PgPool::connect_lazy("postgres://localhost/test").unwrap();
        let op = RunningQueries::new(pool);
        assert_eq!(op.kind(), "postgres");
    }
}
