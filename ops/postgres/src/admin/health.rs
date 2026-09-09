//! Health check operation.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgPool;

use crate::helpers::{pg_error, to_value};

/// Output of [`HealthCheck`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCheckOutput {
    /// Always `true` when the check succeeds.
    pub healthy: bool,
}

/// Verify database connectivity with `SELECT 1`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_postgres::admin::health::HealthCheck;
/// use ironflow_core::operation::Operation;
///
/// # fn example(pool: sqlx::PgPool) {
/// let op = HealthCheck::new(pool);
/// assert_eq!(op.kind(), "postgres");
/// # }
/// ```
pub struct HealthCheck {
    pool: PgPool,
}

impl HealthCheck {
    /// Create a new health-check operation.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] if the database is unreachable.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<HealthCheckOutput, OperationError> {
        sqlx::query("SELECT 1")
            .fetch_one(&self.pool)
            .await
            .map_err(pg_error)?;
        Ok(HealthCheckOutput { healthy: true })
    }
}

#[async_trait]
impl Operation for HealthCheck {
    fn kind(&self) -> &str {
        "postgres"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
}

impl TypedOperation for HealthCheck {
    type Output = HealthCheckOutput;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn kind() {
        let pool = PgPool::connect_lazy("postgres://localhost/test").unwrap();
        let op = HealthCheck::new(pool);
        assert_eq!(op.kind(), "postgres");
    }
}
