//! Constraint introspection operation.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgPool, Row};

use crate::helpers::{pg_error, to_value};

/// A constraint description.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConstraintInfo {
    /// Constraint name.
    pub name: String,
    /// Constraint type (`PRIMARY KEY`, `FOREIGN KEY`, `UNIQUE`, `CHECK`).
    pub constraint_type: String,
    /// Column names involved, if applicable.
    pub columns: Vec<String>,
}

/// Output of [`ListConstraints`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListConstraintsOutput {
    /// Constraints on the table.
    pub constraints: Vec<ConstraintInfo>,
}

/// List the constraints on a table.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_postgres::schema::constraints::ListConstraints;
/// use ironflow_core::operation::Operation;
///
/// # fn example(pool: sqlx::PgPool) {
/// let op = ListConstraints::new(pool, "public", "users");
/// assert_eq!(op.kind(), "postgres");
/// # }
/// ```
pub struct ListConstraints {
    pool: PgPool,
    schema: String,
    table: String,
}

impl ListConstraints {
    /// Create a new list-constraints operation.
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
    pub async fn run(
        &self,
        _ctx: &OperationContext,
    ) -> Result<ListConstraintsOutput, OperationError> {
        let rows = sqlx::query(
            "SELECT c.conname AS constraint_name, \
                    c.contype AS constraint_type, \
                    array_agg(a.attname ORDER BY u.ord) AS columns \
             FROM pg_constraint c \
             JOIN pg_class t ON t.oid = c.conrelid \
             JOIN pg_namespace n ON n.oid = t.relnamespace \
             CROSS JOIN LATERAL unnest(c.conkey) WITH ORDINALITY AS u(attnum, ord) \
             JOIN pg_attribute a ON a.attrelid = t.oid AND a.attnum = u.attnum \
             WHERE n.nspname = $1 AND t.relname = $2 \
             GROUP BY c.conname, c.contype \
             ORDER BY c.conname",
        )
        .bind(&self.schema)
        .bind(&self.table)
        .fetch_all(&self.pool)
        .await
        .map_err(pg_error)?;
        let constraints = rows
            .iter()
            .map(|r| {
                let type_char: String = r
                    .try_get::<String, _>("constraint_type")
                    .map_err(pg_error)?;
                let constraint_type = match type_char.as_str() {
                    "p" => "PRIMARY KEY",
                    "f" => "FOREIGN KEY",
                    "u" => "UNIQUE",
                    "c" => "CHECK",
                    "x" => "EXCLUSION",
                    other => other,
                }
                .to_string();
                Ok(ConstraintInfo {
                    name: r
                        .try_get::<String, _>("constraint_name")
                        .map_err(pg_error)?,
                    constraint_type,
                    columns: r.try_get::<Vec<String>, _>("columns").map_err(pg_error)?,
                })
            })
            .collect::<Result<Vec<_>, OperationError>>()?;
        Ok(ListConstraintsOutput { constraints })
    }
}

#[async_trait]
impl Operation for ListConstraints {
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

impl TypedOperation for ListConstraints {
    type Output = ListConstraintsOutput;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn kind() {
        let pool = PgPool::connect_lazy("postgres://localhost/test").unwrap();
        let op = ListConstraints::new(pool, "public", "users");
        assert_eq!(op.kind(), "postgres");
    }
}
