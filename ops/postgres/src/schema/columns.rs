//! Column introspection operation.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgPool, Row};

use crate::helpers::{pg_error, to_value};

/// A column description.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnInfo {
    /// Column name.
    pub name: String,
    /// Data type (e.g. `integer`, `text`).
    pub data_type: String,
    /// Whether the column is nullable.
    pub is_nullable: bool,
    /// Default value expression, if any.
    pub column_default: Option<String>,
    /// Ordinal position (1-based).
    pub ordinal_position: i32,
}

/// Output of [`ListColumns`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListColumnsOutput {
    /// Columns of the table.
    pub columns: Vec<ColumnInfo>,
}

/// Describe the columns of a table.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_postgres::schema::columns::ListColumns;
/// use ironflow_core::operation::Operation;
///
/// # fn example(pool: sqlx::PgPool) {
/// let op = ListColumns::new(pool, "public", "users");
/// assert_eq!(op.kind(), "postgres");
/// # }
/// ```
pub struct ListColumns {
    pool: PgPool,
    schema: String,
    table: String,
}

impl ListColumns {
    /// Create a new list-columns operation.
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
    pub async fn run(&self, _ctx: &OperationContext) -> Result<ListColumnsOutput, OperationError> {
        let rows = sqlx::query(
            "SELECT column_name, data_type, is_nullable, column_default, ordinal_position \
             FROM information_schema.columns \
             WHERE table_schema = $1 AND table_name = $2 \
             ORDER BY ordinal_position",
        )
        .bind(&self.schema)
        .bind(&self.table)
        .fetch_all(&self.pool)
        .await
        .map_err(pg_error)?;
        let columns = rows
            .iter()
            .map(|r| {
                Ok(ColumnInfo {
                    name: r.try_get::<String, _>("column_name").map_err(pg_error)?,
                    data_type: r.try_get::<String, _>("data_type").map_err(pg_error)?,
                    is_nullable: r.try_get::<String, _>("is_nullable").map_err(pg_error)? == "YES",
                    column_default: r
                        .try_get::<Option<String>, _>("column_default")
                        .map_err(pg_error)?,
                    ordinal_position: r.try_get::<i32, _>("ordinal_position").map_err(pg_error)?,
                })
            })
            .collect::<Result<Vec<_>, OperationError>>()?;
        Ok(ListColumnsOutput { columns })
    }
}

#[async_trait]
impl Operation for ListColumns {
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

impl TypedOperation for ListColumns {
    type Output = ListColumnsOutput;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn kind() {
        let pool = PgPool::connect_lazy("postgres://localhost/test").unwrap();
        let op = ListColumns::new(pool, "public", "users");
        assert_eq!(op.kind(), "postgres");
    }
}
