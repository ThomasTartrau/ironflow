//! Table listing and existence check operations.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgPool, Row};

use crate::helpers::{pg_error, to_value};

/// Output of [`ListTables`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListTablesOutput {
    /// Table names.
    pub tables: Vec<String>,
}

/// List all tables in a given schema.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_postgres::schema::tables::ListTables;
/// use ironflow_core::operation::Operation;
///
/// # fn example(pool: sqlx::PgPool) {
/// let op = ListTables::new(pool, "public");
/// assert_eq!(op.kind(), "postgres");
/// # }
/// ```
pub struct ListTables {
    pool: PgPool,
    schema: String,
}

impl ListTables {
    /// Create a new list-tables operation.
    pub fn new(pool: PgPool, schema: impl Into<String>) -> Self {
        Self {
            pool,
            schema: schema.into(),
        }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] on connection errors.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<ListTablesOutput, OperationError> {
        let rows = sqlx::query(
            "SELECT table_name FROM information_schema.tables \
             WHERE table_schema = $1 AND table_type = 'BASE TABLE' \
             ORDER BY table_name",
        )
        .bind(&self.schema)
        .fetch_all(&self.pool)
        .await
        .map_err(pg_error)?;
        let tables = rows
            .iter()
            .map(|r| r.try_get::<String, _>("table_name").map_err(pg_error))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ListTablesOutput { tables })
    }
}

#[async_trait]
impl Operation for ListTables {
    fn kind(&self) -> &str {
        "postgres"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "schema": self.schema }))
    }
}

impl TypedOperation for ListTables {
    type Output = ListTablesOutput;
}

/// Output of [`TableExists`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableExistsOutput {
    /// Whether the table exists.
    pub exists: bool,
}

/// Check whether a table exists in a schema.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_postgres::schema::tables::TableExists;
/// use ironflow_core::operation::Operation;
///
/// # fn example(pool: sqlx::PgPool) {
/// let op = TableExists::new(pool, "public", "users");
/// assert_eq!(op.kind(), "postgres");
/// # }
/// ```
pub struct TableExists {
    pool: PgPool,
    schema: String,
    table: String,
}

impl TableExists {
    /// Create a new table-exists operation.
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
    pub async fn run(&self, _ctx: &OperationContext) -> Result<TableExistsOutput, OperationError> {
        let row = sqlx::query(
            "SELECT EXISTS( \
                SELECT 1 FROM information_schema.tables \
                WHERE table_schema = $1 AND table_name = $2 \
            ) AS exists",
        )
        .bind(&self.schema)
        .bind(&self.table)
        .fetch_one(&self.pool)
        .await
        .map_err(pg_error)?;
        let exists: bool = row.try_get("exists").map_err(pg_error)?;
        Ok(TableExistsOutput { exists })
    }
}

#[async_trait]
impl Operation for TableExists {
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

impl TypedOperation for TableExists {
    type Output = TableExistsOutput;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn list_tables_kind() {
        let pool = PgPool::connect_lazy("postgres://localhost/test").unwrap();
        let op = ListTables::new(pool, "public");
        assert_eq!(op.kind(), "postgres");
    }

    #[tokio::test]
    async fn table_exists_kind() {
        let pool = PgPool::connect_lazy("postgres://localhost/test").unwrap();
        let op = TableExists::new(pool, "public", "users");
        assert_eq!(op.kind(), "postgres");
    }

    #[tokio::test]
    async fn list_tables_input_no_secrets() {
        let pool = PgPool::connect_lazy("postgres://localhost/test").unwrap();
        let op = ListTables::new(pool, "public");
        let input = op.input().unwrap();
        let text = input.to_string();
        assert!(!text.contains("postgres://"), "leaked URL: {text}");
    }
}
