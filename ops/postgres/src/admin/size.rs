//! Database and table size operations.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgPool, Row};

use crate::helpers::{pg_error, to_value};

/// Output of [`DatabaseSize`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseSizeOutput {
    /// Database name.
    pub database: String,
    /// Size in bytes.
    pub size_bytes: i64,
}

/// Get the size of a database in bytes.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_postgres::admin::size::DatabaseSize;
/// use ironflow_core::operation::Operation;
///
/// # fn example(pool: sqlx::PgPool) {
/// let op = DatabaseSize::new(pool, "mydb");
/// assert_eq!(op.kind(), "postgres");
/// # }
/// ```
pub struct DatabaseSize {
    pool: PgPool,
    database: String,
}

impl DatabaseSize {
    /// Create a new database-size operation.
    pub fn new(pool: PgPool, database: impl Into<String>) -> Self {
        Self {
            pool,
            database: database.into(),
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] on connection errors.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<DatabaseSizeOutput, OperationError> {
        let row = sqlx::query("SELECT pg_database_size($1) AS size")
            .bind(&self.database)
            .fetch_one(&self.pool)
            .await
            .map_err(pg_error)?;
        let size: i64 = row.try_get("size").map_err(pg_error)?;
        Ok(DatabaseSizeOutput {
            database: self.database.clone(),
            size_bytes: size,
        })
    }
}

#[async_trait]
impl Operation for DatabaseSize {
    fn kind(&self) -> &str {
        "postgres"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "database": self.database }))
    }
}

impl TypedOperation for DatabaseSize {
    type Output = DatabaseSizeOutput;
}

/// Output of [`TableSize`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableSizeOutput {
    /// Fully qualified table name.
    pub table: String,
    /// Total size in bytes (table + indexes + toast).
    pub total_bytes: i64,
    /// Table-only size in bytes.
    pub table_bytes: i64,
    /// Index size in bytes.
    pub index_bytes: i64,
}

/// Get the size of a table including indexes.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_postgres::admin::size::TableSize;
/// use ironflow_core::operation::Operation;
///
/// # fn example(pool: sqlx::PgPool) {
/// let op = TableSize::new(pool, "public", "users");
/// assert_eq!(op.kind(), "postgres");
/// # }
/// ```
pub struct TableSize {
    pool: PgPool,
    schema: String,
    table: String,
}

impl TableSize {
    /// Create a new table-size operation.
    pub fn new(pool: PgPool, schema: impl Into<String>, table: impl Into<String>) -> Self {
        Self {
            pool,
            schema: schema.into(),
            table: table.into(),
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] on connection errors.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<TableSizeOutput, OperationError> {
        let qualified = format!("{}.{}", self.schema, self.table);
        let row = sqlx::query(
            "SELECT pg_total_relation_size($1::regclass) AS total, \
                    pg_table_size($1::regclass) AS tbl, \
                    pg_indexes_size($1::regclass) AS idx",
        )
        .bind(&qualified)
        .fetch_one(&self.pool)
        .await
        .map_err(pg_error)?;
        Ok(TableSizeOutput {
            table: qualified,
            total_bytes: row.try_get("total").map_err(pg_error)?,
            table_bytes: row.try_get("tbl").map_err(pg_error)?,
            index_bytes: row.try_get("idx").map_err(pg_error)?,
        })
    }
}

#[async_trait]
impl Operation for TableSize {
    fn kind(&self) -> &str {
        "postgres"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "schema": self.schema, "table": self.table }))
    }
}

impl TypedOperation for TableSize {
    type Output = TableSizeOutput;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn database_size_kind() {
        let pool = PgPool::connect_lazy("postgres://localhost/test").unwrap();
        let op = DatabaseSize::new(pool, "test");
        assert_eq!(op.kind(), "postgres");
    }

    #[tokio::test]
    async fn table_size_kind() {
        let pool = PgPool::connect_lazy("postgres://localhost/test").unwrap();
        let op = TableSize::new(pool, "public", "users");
        assert_eq!(op.kind(), "postgres");
    }
}
