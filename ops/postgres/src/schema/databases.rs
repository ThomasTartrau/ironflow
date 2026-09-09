//! Database listing operation.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgPool, Row};

use crate::helpers::{pg_error, to_value};

/// Output of [`ListDatabases`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListDatabasesOutput {
    /// Database names.
    pub databases: Vec<String>,
}

/// List all databases on the server.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_postgres::schema::databases::ListDatabases;
/// use ironflow_core::operation::Operation;
///
/// # fn example(pool: sqlx::PgPool) {
/// let op = ListDatabases::new(pool);
/// assert_eq!(op.kind(), "postgres");
/// # }
/// ```
pub struct ListDatabases {
    pool: PgPool,
}

impl ListDatabases {
    /// Create a new list-databases operation.
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
    ) -> Result<ListDatabasesOutput, OperationError> {
        let rows = sqlx::query(
            "SELECT datname FROM pg_database WHERE datistemplate = false ORDER BY datname",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(pg_error)?;
        let databases = rows
            .iter()
            .map(|r| r.try_get::<String, _>("datname").map_err(pg_error))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ListDatabasesOutput { databases })
    }
}

#[async_trait]
impl Operation for ListDatabases {
    fn kind(&self) -> &str {
        "postgres"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
}

impl TypedOperation for ListDatabases {
    type Output = ListDatabasesOutput;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn kind() {
        let pool = PgPool::connect_lazy("postgres://localhost/test").unwrap();
        let op = ListDatabases::new(pool);
        assert_eq!(op.kind(), "postgres");
    }
}
