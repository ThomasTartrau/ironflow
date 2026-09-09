//! Maintenance operations: `VACUUM`, `ANALYZE`, `REINDEX`.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{Executor, PgPool};

use crate::helpers::{pg_error, quote_identifier, to_value};

/// Output of maintenance operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaintenanceOutput {
    /// The operation that was performed.
    pub operation: String,
    /// The target (table or index name).
    pub target: String,
}

/// Run `VACUUM` on a table.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_postgres::maintenance::Vacuum;
/// use ironflow_core::operation::Operation;
///
/// # fn example(pool: sqlx::PgPool) {
/// let op = Vacuum::new(pool, "public", "users");
/// assert_eq!(op.kind(), "postgres");
/// # }
/// ```
pub struct Vacuum {
    pool: PgPool,
    schema: String,
    table: String,
}

impl Vacuum {
    /// Create a new vacuum operation.
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
    /// Returns [`OperationError::External`] on connection or permission errors.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<MaintenanceOutput, OperationError> {
        let schema = quote_identifier(&self.schema)?;
        let table = quote_identifier(&self.table)?;
        let sql = format!("VACUUM {schema}.{table}");
        self.pool.execute(sql.as_str()).await.map_err(pg_error)?;
        Ok(MaintenanceOutput {
            operation: "VACUUM".to_string(),
            target: format!("{}.{}", self.schema, self.table),
        })
    }
}

#[async_trait]
impl Operation for Vacuum {
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

impl TypedOperation for Vacuum {
    type Output = MaintenanceOutput;
}

/// Run `ANALYZE` on a table.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_postgres::maintenance::Analyze;
/// use ironflow_core::operation::Operation;
///
/// # fn example(pool: sqlx::PgPool) {
/// let op = Analyze::new(pool, "public", "users");
/// assert_eq!(op.kind(), "postgres");
/// # }
/// ```
pub struct Analyze {
    pool: PgPool,
    schema: String,
    table: String,
}

impl Analyze {
    /// Create a new analyze operation.
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
    /// Returns [`OperationError::External`] on connection or permission errors.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<MaintenanceOutput, OperationError> {
        let schema = quote_identifier(&self.schema)?;
        let table = quote_identifier(&self.table)?;
        let sql = format!("ANALYZE {schema}.{table}");
        self.pool.execute(sql.as_str()).await.map_err(pg_error)?;
        Ok(MaintenanceOutput {
            operation: "ANALYZE".to_string(),
            target: format!("{}.{}", self.schema, self.table),
        })
    }
}

#[async_trait]
impl Operation for Analyze {
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

impl TypedOperation for Analyze {
    type Output = MaintenanceOutput;
}

/// Run `REINDEX TABLE` on a table.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_postgres::maintenance::Reindex;
/// use ironflow_core::operation::Operation;
///
/// # fn example(pool: sqlx::PgPool) {
/// let op = Reindex::new(pool, "public", "users");
/// assert_eq!(op.kind(), "postgres");
/// # }
/// ```
pub struct Reindex {
    pool: PgPool,
    schema: String,
    table: String,
}

impl Reindex {
    /// Create a new reindex operation.
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
    /// Returns [`OperationError::External`] on connection or permission errors.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<MaintenanceOutput, OperationError> {
        let schema = quote_identifier(&self.schema)?;
        let table = quote_identifier(&self.table)?;
        let sql = format!("REINDEX TABLE {schema}.{table}");
        self.pool.execute(sql.as_str()).await.map_err(pg_error)?;
        Ok(MaintenanceOutput {
            operation: "REINDEX".to_string(),
            target: format!("{}.{}", self.schema, self.table),
        })
    }
}

#[async_trait]
impl Operation for Reindex {
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

impl TypedOperation for Reindex {
    type Output = MaintenanceOutput;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn vacuum_kind() {
        let pool = PgPool::connect_lazy("postgres://localhost/test").unwrap();
        let op = Vacuum::new(pool, "public", "users");
        assert_eq!(op.kind(), "postgres");
    }

    #[tokio::test]
    async fn analyze_kind() {
        let pool = PgPool::connect_lazy("postgres://localhost/test").unwrap();
        let op = Analyze::new(pool, "public", "users");
        assert_eq!(op.kind(), "postgres");
    }

    #[tokio::test]
    async fn reindex_kind() {
        let pool = PgPool::connect_lazy("postgres://localhost/test").unwrap();
        let op = Reindex::new(pool, "public", "users");
        assert_eq!(op.kind(), "postgres");
    }

    #[tokio::test]
    async fn vacuum_input_no_secrets() {
        let pool = PgPool::connect_lazy("postgres://localhost/test").unwrap();
        let op = Vacuum::new(pool, "public", "users");
        let input = op.input().unwrap();
        let text = input.to_string();
        assert!(!text.contains("postgres://"), "leaked URL: {text}");
    }
}
