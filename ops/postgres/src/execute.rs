//! Mutating operations: `INSERT`, `UPDATE`, `DELETE`, batch execution, and
//! transactions.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{Executor, PgPool};

use crate::helpers::{bind_json_param, pg_error, to_value};

/// Output of [`Execute`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteOutput {
    /// Number of rows affected by the statement.
    pub rows_affected: u64,
}

/// Execute a single mutating SQL statement (`INSERT`, `UPDATE`, `DELETE`).
///
/// Returns the number of rows affected.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_postgres::execute::Execute;
/// use ironflow_core::operation::Operation;
///
/// # fn example(pool: sqlx::PgPool) {
/// let op = Execute::new(pool, "INSERT INTO users (name) VALUES ($1)", vec![serde_json::json!("Alice")]);
/// assert_eq!(op.kind(), "postgres");
/// # }
/// ```
pub struct Execute {
    pool: PgPool,
    sql: String,
    params: Vec<Value>,
}

impl Execute {
    /// Create a new execute operation.
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
    pub async fn run(&self, _ctx: &OperationContext) -> Result<ExecuteOutput, OperationError> {
        let mut query = sqlx::query(&self.sql);
        for p in &self.params {
            query = bind_json_param(query, p);
        }
        let result = query.execute(&self.pool).await.map_err(pg_error)?;
        Ok(ExecuteOutput {
            rows_affected: result.rows_affected(),
        })
    }
}

#[async_trait]
impl Operation for Execute {
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

impl TypedOperation for Execute {
    type Output = ExecuteOutput;
}

/// Output of [`ExecuteBatch`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteBatchOutput {
    /// Number of rows affected by each statement, in order.
    pub rows_affected: Vec<u64>,
}

/// Execute multiple SQL statements in sequence (not transactional).
///
/// Each statement is executed independently. If a statement fails, the
/// remaining statements are not executed.
///
/// # Safety
///
/// Statements are executed as raw SQL. Never build them from untrusted input
/// without proper parameterization. Use [`Execute`] with bind parameters for
/// user-supplied values.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_postgres::execute::ExecuteBatch;
/// use ironflow_core::operation::Operation;
///
/// # fn example(pool: sqlx::PgPool) {
/// let op = ExecuteBatch::new(pool, vec![
///     "CREATE TABLE IF NOT EXISTS t (id int)".to_string(),
///     "INSERT INTO t VALUES (1)".to_string(),
/// ]);
/// assert_eq!(op.kind(), "postgres");
/// # }
/// ```
pub struct ExecuteBatch {
    pool: PgPool,
    statements: Vec<String>,
}

impl ExecuteBatch {
    /// Create a new batch-execute operation.
    pub fn new(pool: PgPool, statements: Vec<String>) -> Self {
        Self { pool, statements }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] on SQL or connection errors.
    /// Stops at the first failing statement.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<ExecuteBatchOutput, OperationError> {
        let mut results = Vec::with_capacity(self.statements.len());
        for stmt in &self.statements {
            let result = self.pool.execute(stmt.as_str()).await.map_err(pg_error)?;
            results.push(result.rows_affected());
        }
        Ok(ExecuteBatchOutput {
            rows_affected: results,
        })
    }
}

#[async_trait]
impl Operation for ExecuteBatch {
    fn kind(&self) -> &str {
        "postgres"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "statements": self.statements }))
    }
}

impl TypedOperation for ExecuteBatch {
    type Output = ExecuteBatchOutput;
}

/// Output of [`Transaction`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionOutput {
    /// Number of rows affected by each statement, in order.
    pub rows_affected: Vec<u64>,
}

/// Execute multiple SQL statements inside an atomic transaction.
///
/// All statements succeed together or are rolled back on the first error.
///
/// # Safety
///
/// Statements are executed as raw SQL. Never build them from untrusted input
/// without proper parameterization. Use [`Execute`] with bind parameters for
/// user-supplied values.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_postgres::execute::Transaction;
/// use ironflow_core::operation::Operation;
///
/// # fn example(pool: sqlx::PgPool) {
/// let op = Transaction::new(pool, vec![
///     "INSERT INTO accounts (id, balance) VALUES (1, 100)".to_string(),
///     "UPDATE accounts SET balance = balance - 50 WHERE id = 1".to_string(),
/// ]);
/// assert_eq!(op.kind(), "postgres");
/// # }
/// ```
pub struct Transaction {
    pool: PgPool,
    statements: Vec<String>,
}

impl Transaction {
    /// Create a new transaction operation.
    pub fn new(pool: PgPool, statements: Vec<String>) -> Self {
        Self { pool, statements }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] on SQL or connection errors.
    /// The transaction is rolled back on any failure.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<TransactionOutput, OperationError> {
        let mut tx = self.pool.begin().await.map_err(pg_error)?;
        let mut results = Vec::with_capacity(self.statements.len());
        for stmt in &self.statements {
            let result = tx.execute(stmt.as_str()).await.map_err(pg_error)?;
            results.push(result.rows_affected());
        }
        tx.commit().await.map_err(pg_error)?;
        Ok(TransactionOutput {
            rows_affected: results,
        })
    }
}

#[async_trait]
impl Operation for Transaction {
    fn kind(&self) -> &str {
        "postgres"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "statements": self.statements }))
    }
}

impl TypedOperation for Transaction {
    type Output = TransactionOutput;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn execute_kind() {
        let pool = PgPool::connect_lazy("postgres://localhost/test").unwrap();
        let op = Execute::new(pool, "INSERT INTO t VALUES (1)", vec![]);
        assert_eq!(op.kind(), "postgres");
    }

    #[tokio::test]
    async fn execute_batch_kind() {
        let pool = PgPool::connect_lazy("postgres://localhost/test").unwrap();
        let op = ExecuteBatch::new(pool, vec!["SELECT 1".to_string()]);
        assert_eq!(op.kind(), "postgres");
    }

    #[tokio::test]
    async fn transaction_kind() {
        let pool = PgPool::connect_lazy("postgres://localhost/test").unwrap();
        let op = Transaction::new(pool, vec!["SELECT 1".to_string()]);
        assert_eq!(op.kind(), "postgres");
    }

    #[tokio::test]
    async fn execute_input_no_secrets() {
        let pool = PgPool::connect_lazy("postgres://localhost/test").unwrap();
        let op = Execute::new(pool, "INSERT INTO t VALUES ($1)", vec![]);
        let input = op.input().unwrap();
        let text = input.to_string();
        assert!(!text.contains("postgres://"), "leaked URL: {text}");
    }

    #[tokio::test]
    async fn execute_batch_input_no_secrets() {
        let pool = PgPool::connect_lazy("postgres://localhost/test").unwrap();
        let op = ExecuteBatch::new(pool, vec!["SELECT 1".to_string()]);
        let input = op.input().unwrap();
        let text = input.to_string();
        assert!(!text.contains("postgres://"), "leaked URL: {text}");
    }

    #[tokio::test]
    async fn transaction_input_no_secrets() {
        let pool = PgPool::connect_lazy("postgres://localhost/test").unwrap();
        let op = Transaction::new(pool, vec!["SELECT 1".to_string()]);
        let input = op.input().unwrap();
        let text = input.to_string();
        assert!(!text.contains("postgres://"), "leaked URL: {text}");
    }
}
