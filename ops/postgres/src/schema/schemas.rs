//! Schema listing operation.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgPool, Row};

use crate::helpers::{pg_error, to_value};

/// Output of [`ListSchemas`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListSchemasOutput {
    /// Schema names.
    pub schemas: Vec<String>,
}

/// List all schemas in the current database.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_postgres::schema::schemas::ListSchemas;
/// use ironflow_core::operation::Operation;
///
/// # fn example(pool: sqlx::PgPool) {
/// let op = ListSchemas::new(pool);
/// assert_eq!(op.kind(), "postgres");
/// # }
/// ```
pub struct ListSchemas {
    pool: PgPool,
}

impl ListSchemas {
    /// Create a new list-schemas operation.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] on connection errors.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<ListSchemasOutput, OperationError> {
        let rows =
            sqlx::query("SELECT schema_name FROM information_schema.schemata ORDER BY schema_name")
                .fetch_all(&self.pool)
                .await
                .map_err(pg_error)?;
        let schemas = rows
            .iter()
            .map(|r| r.try_get::<String, _>("schema_name").map_err(pg_error))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ListSchemasOutput { schemas })
    }
}

#[async_trait]
impl Operation for ListSchemas {
    fn kind(&self) -> &str {
        "postgres"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
}

impl TypedOperation for ListSchemas {
    type Output = ListSchemasOutput;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn kind() {
        let pool = PgPool::connect_lazy("postgres://localhost/test").unwrap();
        let op = ListSchemas::new(pool);
        assert_eq!(op.kind(), "postgres");
    }
}
