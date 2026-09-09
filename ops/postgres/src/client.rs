//! [`PostgresClient`] -- connection pool wrapper for PostgreSQL operations.

use std::fmt;

use ironflow_core::error::OperationError;
use ironflow_core::operation::OperationContext;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

/// A PostgreSQL client wrapping a [`PgPool`].
///
/// Holds a connection pool that is shared across all operations created from
/// this client. The pool is created once and reused for the lifetime of the
/// client.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_postgres::PostgresClient;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let client = PostgresClient::connect("postgres://localhost/mydb").await?;
/// let pool = client.pool();
/// # Ok(())
/// # }
/// ```
pub struct PostgresClient {
    pool: PgPool,
}

impl PostgresClient {
    /// Connect to a PostgreSQL database using the given connection URL.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] if the connection pool cannot be
    /// created (invalid URL, unreachable server, authentication failure).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_postgres::PostgresClient;
    ///
    /// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
    /// let client = PostgresClient::connect("postgres://localhost/mydb").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn connect(url: &str) -> Result<Self, OperationError> {
        if url.is_empty() {
            return Err(OperationError::External {
                origin: "postgres".to_string(),
                message: "connection URL must not be empty".to_string(),
            });
        }
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(url)
            .await
            .map_err(|e| OperationError::External {
                origin: "postgres".to_string(),
                message: e.to_string(),
            })?;
        Ok(Self { pool })
    }

    /// Create a client from an existing [`PgPool`].
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_postgres::PostgresClient;
    /// use sqlx::PgPool;
    ///
    /// # async fn example() -> Result<(), sqlx::Error> {
    /// let pool = PgPool::connect("postgres://localhost/mydb").await?;
    /// let client = PostgresClient::from_pool(pool);
    /// # Ok(())
    /// # }
    /// ```
    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Create a client by reading `postgres_url` from the secret store.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Secret`] if the secret store fails, or
    /// [`OperationError::External`] if the secret is missing or the connection
    /// cannot be established.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_postgres::PostgresClient;
    /// use ironflow_core::operation::{OperationContext, NoopSecretResolver};
    /// use std::sync::Arc;
    ///
    /// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
    /// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
    /// let client = PostgresClient::from_context(&ctx).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn from_context(ctx: &OperationContext) -> Result<Self, OperationError> {
        let secret = ctx.secrets().get("postgres_url").await?;
        let url = secret
            .ok_or_else(|| OperationError::External {
                origin: "postgres".to_string(),
                message: "secret 'postgres_url' not found".to_string(),
            })?
            .value;
        Self::connect(&url).await
    }

    /// Returns a reference to the underlying connection pool.
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

impl fmt::Debug for PostgresClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PostgresClient")
            .field("url", &"[REDACTED]")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ironflow_core::operation::{NoopSecretResolver, OperationContext};

    use super::*;

    #[tokio::test]
    async fn debug_does_not_leak() {
        let client = PostgresClient {
            pool: PgPool::connect_lazy("postgres://user:pass@localhost/db").unwrap(),
        };
        let debug = format!("{client:?}");
        assert!(!debug.contains("user"), "leaked user: {debug}");
        assert!(!debug.contains("pass"), "leaked password: {debug}");
        assert!(!debug.contains("localhost"), "leaked host: {debug}");
        assert!(debug.contains("REDACTED"), "missing redaction: {debug}");
    }

    #[tokio::test]
    async fn connect_empty_url() {
        let err = PostgresClient::connect("").await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("must not be empty"), "unexpected error: {msg}");
    }

    #[tokio::test]
    async fn from_context_missing_secret() {
        let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
        let err = PostgresClient::from_context(&ctx).await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("postgres_url"), "unexpected error: {msg}");
    }
}
