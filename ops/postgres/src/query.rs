//! Query operations: `SELECT` statements returning rows or scalar values.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgPool, Row as _};

use crate::helpers::{bind_json_param, column_to_json, pg_error, row_to_json, to_value};

/// Output of [`QueryRows`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryRowsOutput {
    /// The rows as a JSON array. Each row is a JSON object with column
    /// names as keys.
    pub rows: Vec<Value>,
}

/// Execute a `SELECT` query and return all matching rows as JSON.
///
/// Parameters are passed as a JSON array and bound positionally (`$1`, `$2`, ...).
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_postgres::query::QueryRows;
/// use ironflow_core::operation::Operation;
///
/// # fn example(pool: sqlx::PgPool) {
/// let op = QueryRows::new(pool, "SELECT id, name FROM users WHERE active = $1", vec![serde_json::json!(true)]);
/// assert_eq!(op.kind(), "postgres");
/// # }
/// ```
pub struct QueryRows {
    pool: PgPool,
    sql: String,
    params: Vec<Value>,
}

impl QueryRows {
    /// Create a new query-rows operation.
    pub fn new(pool: PgPool, sql: impl Into<String>, params: Vec<Value>) -> Self {
        Self {
            pool,
            sql: sql.into(),
            params,
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] on SQL or connection errors.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<QueryRowsOutput, OperationError> {
        let mut query = sqlx::query(&self.sql);
        for p in &self.params {
            query = bind_json_param(query, p);
        }
        let rows = query.fetch_all(&self.pool).await.map_err(pg_error)?;
        let json_rows: Vec<Value> = rows.iter().map(row_to_json).collect::<Result<_, _>>()?;
        Ok(QueryRowsOutput { rows: json_rows })
    }
}

#[async_trait]
impl Operation for QueryRows {
    fn kind(&self) -> &str {
        "postgres"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "sql": self.sql, "params": self.params }))
    }
}

impl TypedOperation for QueryRows {
    type Output = QueryRowsOutput;
}

/// Output of [`QueryOne`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryOneOutput {
    /// The single row as a JSON object.
    pub row: Value,
}

/// Execute a `SELECT` query and return exactly one row.
///
/// # Errors
///
/// Returns an error if the query returns zero rows or more than one row.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_postgres::query::QueryOne;
/// use ironflow_core::operation::Operation;
///
/// # fn example(pool: sqlx::PgPool) {
/// let op = QueryOne::new(pool, "SELECT id, name FROM users WHERE id = $1", vec![serde_json::json!(1)]);
/// assert_eq!(op.kind(), "postgres");
/// # }
/// ```
pub struct QueryOne {
    pool: PgPool,
    sql: String,
    params: Vec<Value>,
}

impl QueryOne {
    /// Create a new query-one operation.
    pub fn new(pool: PgPool, sql: impl Into<String>, params: Vec<Value>) -> Self {
        Self {
            pool,
            sql: sql.into(),
            params,
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] if the query returns zero rows,
    /// more than one row, or on SQL/connection errors.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<QueryOneOutput, OperationError> {
        let mut query = sqlx::query(&self.sql);
        for p in &self.params {
            query = bind_json_param(query, p);
        }
        let row = query.fetch_one(&self.pool).await.map_err(pg_error)?;
        let json = row_to_json(&row)?;
        Ok(QueryOneOutput { row: json })
    }
}

#[async_trait]
impl Operation for QueryOne {
    fn kind(&self) -> &str {
        "postgres"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "sql": self.sql, "params": self.params }))
    }
}

impl TypedOperation for QueryOne {
    type Output = QueryOneOutput;
}

/// Output of [`QueryScalar`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryScalarOutput {
    /// The scalar value as JSON.
    pub value: Value,
}

/// Execute a `SELECT` query and return a single scalar value.
///
/// The query must return exactly one row with one column.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_postgres::query::QueryScalar;
/// use ironflow_core::operation::Operation;
///
/// # fn example(pool: sqlx::PgPool) {
/// let op = QueryScalar::new(pool, "SELECT count(*) FROM users", vec![]);
/// assert_eq!(op.kind(), "postgres");
/// # }
/// ```
pub struct QueryScalar {
    pool: PgPool,
    sql: String,
    params: Vec<Value>,
}

impl QueryScalar {
    /// Create a new query-scalar operation.
    pub fn new(pool: PgPool, sql: impl Into<String>, params: Vec<Value>) -> Self {
        Self {
            pool,
            sql: sql.into(),
            params,
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] on SQL/connection errors or if the
    /// query does not return exactly one row with one column.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<QueryScalarOutput, OperationError> {
        let mut query = sqlx::query(&self.sql);
        for p in &self.params {
            query = bind_json_param(query, p);
        }
        let row = query.fetch_one(&self.pool).await.map_err(pg_error)?;
        let columns = row.columns();
        if columns.is_empty() {
            return Err(OperationError::External {
                origin: "postgres".to_string(),
                message: "query returned no columns".to_string(),
            });
        }
        let val = column_to_json(&row, 0)?;
        Ok(QueryScalarOutput { value: val })
    }
}

#[async_trait]
impl Operation for QueryScalar {
    fn kind(&self) -> &str {
        "postgres"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "sql": self.sql, "params": self.params }))
    }
}

impl TypedOperation for QueryScalar {
    type Output = QueryScalarOutput;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn query_rows_kind() {
        let pool = PgPool::connect_lazy("postgres://localhost/test").unwrap();
        let op = QueryRows::new(pool, "SELECT 1", vec![]);
        assert_eq!(op.kind(), "postgres");
    }

    #[tokio::test]
    async fn query_one_kind() {
        let pool = PgPool::connect_lazy("postgres://localhost/test").unwrap();
        let op = QueryOne::new(pool, "SELECT 1", vec![]);
        assert_eq!(op.kind(), "postgres");
    }

    #[tokio::test]
    async fn query_scalar_kind() {
        let pool = PgPool::connect_lazy("postgres://localhost/test").unwrap();
        let op = QueryScalar::new(pool, "SELECT 1", vec![]);
        assert_eq!(op.kind(), "postgres");
    }

    #[tokio::test]
    async fn query_rows_input_no_secrets() {
        let pool = PgPool::connect_lazy("postgres://localhost/test").unwrap();
        let op = QueryRows::new(pool, "SELECT 1", vec![]);
        let input = op.input().unwrap();
        let text = input.to_string();
        assert!(!text.contains("password"), "leaked secret: {text}");
        assert!(!text.contains("postgres://"), "leaked URL: {text}");
    }

    #[tokio::test]
    async fn query_one_input_no_secrets() {
        let pool = PgPool::connect_lazy("postgres://localhost/test").unwrap();
        let op = QueryOne::new(pool, "SELECT 1", vec![]);
        let input = op.input().unwrap();
        let text = input.to_string();
        assert!(!text.contains("password"), "leaked secret: {text}");
    }

    #[tokio::test]
    async fn query_scalar_input_no_secrets() {
        let pool = PgPool::connect_lazy("postgres://localhost/test").unwrap();
        let op = QueryScalar::new(pool, "SELECT 1", vec![]);
        let input = op.input().unwrap();
        let text = input.to_string();
        assert!(!text.contains("password"), "leaked secret: {text}");
    }
}
