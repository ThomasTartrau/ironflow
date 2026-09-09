//! Index introspection operation.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgPool, Row};

use crate::helpers::{pg_error, to_value};

/// An index description.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexInfo {
    /// Index name.
    pub name: String,
    /// Whether the index enforces uniqueness.
    pub is_unique: bool,
    /// Whether this is the primary key index.
    pub is_primary: bool,
    /// Index definition (the `CREATE INDEX` statement).
    pub definition: String,
}

/// Output of [`ListIndexes`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListIndexesOutput {
    /// Indexes on the table.
    pub indexes: Vec<IndexInfo>,
}

/// List the indexes on a table.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_postgres::schema::indexes::ListIndexes;
/// use ironflow_core::operation::Operation;
///
/// # fn example(pool: sqlx::PgPool) {
/// let op = ListIndexes::new(pool, "public", "users");
/// assert_eq!(op.kind(), "postgres");
/// # }
/// ```
pub struct ListIndexes {
    pool: PgPool,
    schema: String,
    table: String,
}

impl ListIndexes {
    /// Create a new list-indexes operation.
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
    pub async fn run(&self, _ctx: &OperationContext) -> Result<ListIndexesOutput, OperationError> {
        let rows = sqlx::query(
            "SELECT i.relname AS index_name, \
                    ix.indisunique AS is_unique, \
                    ix.indisprimary AS is_primary, \
                    pg_get_indexdef(ix.indexrelid) AS definition \
             FROM pg_index ix \
             JOIN pg_class i ON i.oid = ix.indexrelid \
             JOIN pg_class t ON t.oid = ix.indrelid \
             JOIN pg_namespace n ON n.oid = t.relnamespace \
             WHERE n.nspname = $1 AND t.relname = $2 \
             ORDER BY i.relname",
        )
        .bind(&self.schema)
        .bind(&self.table)
        .fetch_all(&self.pool)
        .await
        .map_err(pg_error)?;
        let indexes = rows
            .iter()
            .map(|r| {
                Ok(IndexInfo {
                    name: r.try_get::<String, _>("index_name").map_err(pg_error)?,
                    is_unique: r.try_get::<bool, _>("is_unique").map_err(pg_error)?,
                    is_primary: r.try_get::<bool, _>("is_primary").map_err(pg_error)?,
                    definition: r.try_get::<String, _>("definition").map_err(pg_error)?,
                })
            })
            .collect::<Result<Vec<_>, OperationError>>()?;
        Ok(ListIndexesOutput { indexes })
    }
}

#[async_trait]
impl Operation for ListIndexes {
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

impl TypedOperation for ListIndexes {
    type Output = ListIndexesOutput;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn kind() {
        let pool = PgPool::connect_lazy("postgres://localhost/test").unwrap();
        let op = ListIndexes::new(pool, "public", "users");
        assert_eq!(op.kind(), "postgres");
    }
}
