//! Backend process management operations.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgPool, Row};

use crate::helpers::{pg_error, to_value};

/// Output of [`CancelQuery`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CancelQueryOutput {
    /// Whether the cancellation signal was sent successfully.
    pub cancelled: bool,
}

/// Cancel a running query by its backend process ID.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_postgres::admin::process::CancelQuery;
/// use ironflow_core::operation::Operation;
///
/// # fn example(pool: sqlx::PgPool) {
/// let op = CancelQuery::new(pool, 12345);
/// assert_eq!(op.kind(), "postgres");
/// # }
/// ```
pub struct CancelQuery {
    pool: PgPool,
    pid: i32,
}

impl CancelQuery {
    /// Create a new cancel-query operation.
    pub fn new(pool: PgPool, pid: i32) -> Self {
        Self { pool, pid }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] on connection errors.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<CancelQueryOutput, OperationError> {
        let row = sqlx::query("SELECT pg_cancel_backend($1) AS cancelled")
            .bind(self.pid)
            .fetch_one(&self.pool)
            .await
            .map_err(pg_error)?;
        let cancelled: bool = row.try_get("cancelled").map_err(pg_error)?;
        Ok(CancelQueryOutput { cancelled })
    }
}

#[async_trait]
impl Operation for CancelQuery {
    fn kind(&self) -> &str {
        "postgres"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "pid": self.pid }))
    }
}

impl TypedOperation for CancelQuery {
    type Output = CancelQueryOutput;
}

/// Output of [`TerminateBackend`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminateBackendOutput {
    /// Whether the backend was terminated.
    pub terminated: bool,
}

/// Terminate a backend process by its PID.
///
/// This is more forceful than [`CancelQuery`] and should be used as a last
/// resort.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_postgres::admin::process::TerminateBackend;
/// use ironflow_core::operation::Operation;
///
/// # fn example(pool: sqlx::PgPool) {
/// let op = TerminateBackend::new(pool, 12345);
/// assert_eq!(op.kind(), "postgres");
/// # }
/// ```
pub struct TerminateBackend {
    pool: PgPool,
    pid: i32,
}

impl TerminateBackend {
    /// Create a new terminate-backend operation.
    pub fn new(pool: PgPool, pid: i32) -> Self {
        Self { pool, pid }
    }

    /// Execute and return a typed result.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] on connection errors.
    pub async fn run(
        &self,
        _ctx: &OperationContext,
    ) -> Result<TerminateBackendOutput, OperationError> {
        let row = sqlx::query("SELECT pg_terminate_backend($1) AS terminated")
            .bind(self.pid)
            .fetch_one(&self.pool)
            .await
            .map_err(pg_error)?;
        let terminated: bool = row.try_get("terminated").map_err(pg_error)?;
        Ok(TerminateBackendOutput { terminated })
    }
}

#[async_trait]
impl Operation for TerminateBackend {
    fn kind(&self) -> &str {
        "postgres"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({ "pid": self.pid }))
    }
}

impl TypedOperation for TerminateBackend {
    type Output = TerminateBackendOutput;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cancel_query_kind() {
        let pool = PgPool::connect_lazy("postgres://localhost/test").unwrap();
        let op = CancelQuery::new(pool, 123);
        assert_eq!(op.kind(), "postgres");
    }

    #[tokio::test]
    async fn terminate_backend_kind() {
        let pool = PgPool::connect_lazy("postgres://localhost/test").unwrap();
        let op = TerminateBackend::new(pool, 123);
        assert_eq!(op.kind(), "postgres");
    }

    #[tokio::test]
    async fn cancel_query_input_no_secrets() {
        let pool = PgPool::connect_lazy("postgres://localhost/test").unwrap();
        let op = CancelQuery::new(pool, 123);
        let input = op.input().unwrap();
        let text = input.to_string();
        assert!(!text.contains("postgres://"), "leaked URL: {text}");
    }
}
