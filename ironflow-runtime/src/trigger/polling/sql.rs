//! SQL polling probe.
//!
//! [`SqlProbe`] executes a SQL query against a PostgreSQL database at each
//! interval and triggers when the result set is non-empty. The query is
//! responsible for marking rows as processed (e.g. via `UPDATE ... SET
//! processed = true ... RETURNING *`).
//!
//! # Feature flag
//!
//! This module is only available when the `trigger-polling-sql` feature is
//! enabled.
//!
//! # Examples
//!
//! ```no_run
//! use ironflow_runtime::trigger::polling::sql::{SqlProbe, SqlProbeConfig};
//!
//! let probe = SqlProbe::new(SqlProbeConfig {
//!     connection_string: "host=localhost dbname=mydb".to_string(),
//!     query: "SELECT * FROM events WHERE processed = false LIMIT 10".to_string(),
//! });
//! ```

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::spawn;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_postgres::types::Type;
use tokio_postgres::{Client, NoTls, Row};
use tracing::warn;

use super::{PollingProbe, ProbeError, ProbeFuture, ProbeResult};

/// Configuration for a [`SqlProbe`].
///
/// # Examples
///
/// ```
/// use ironflow_runtime::trigger::polling::sql::SqlProbeConfig;
///
/// let config = SqlProbeConfig {
///     connection_string: "host=localhost dbname=app".to_string(),
///     query: "SELECT id FROM jobs WHERE status = 'pending'".to_string(),
/// };
/// assert!(config.query.contains("pending"));
/// ```
#[derive(Debug, Clone)]
pub struct SqlProbeConfig {
    /// PostgreSQL connection string (libpq format).
    pub connection_string: String,
    /// SQL query to execute. Must return rows when there is work to do.
    pub query: String,
}

struct SqlConnection {
    client: Client,
    handle: JoinHandle<()>,
}

/// A SQL polling probe that triggers when a query returns rows.
///
/// The probe reuses a single connection across poll cycles. If the connection
/// drops, it reconnects automatically on the next poll.
///
/// # Examples
///
/// ```no_run
/// use ironflow_runtime::trigger::polling::sql::{SqlProbe, SqlProbeConfig};
/// use ironflow_runtime::trigger::polling::PollingProbe;
///
/// let probe = SqlProbe::new(SqlProbeConfig {
///     connection_string: "host=localhost dbname=mydb".to_string(),
///     query: "SELECT 1".to_string(),
/// });
/// assert_eq!(probe.name(), "sql");
/// ```
pub struct SqlProbe {
    config: SqlProbeConfig,
    conn: Mutex<Option<SqlConnection>>,
}

impl SqlProbe {
    /// Create a new SQL probe with the given configuration.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_runtime::trigger::polling::sql::{SqlProbe, SqlProbeConfig};
    ///
    /// let probe = SqlProbe::new(SqlProbeConfig {
    ///     connection_string: "host=localhost".to_string(),
    ///     query: "SELECT * FROM events".to_string(),
    /// });
    /// ```
    pub fn new(config: SqlProbeConfig) -> Self {
        Self {
            config,
            conn: Mutex::new(None),
        }
    }

    /// Convert a single column value to JSON using the column's Postgres type.
    fn column_to_json(row: &Row, index: usize, col_type: &Type) -> Value {
        match *col_type {
            Type::BOOL => row
                .try_get::<_, Option<bool>>(index)
                .ok()
                .flatten()
                .map_or(Value::Null, |v| json!(v)),
            Type::INT2 => row
                .try_get::<_, Option<i16>>(index)
                .ok()
                .flatten()
                .map_or(Value::Null, |v| json!(v)),
            Type::INT4 => row
                .try_get::<_, Option<i32>>(index)
                .ok()
                .flatten()
                .map_or(Value::Null, |v| json!(v)),
            Type::INT8 => row
                .try_get::<_, Option<i64>>(index)
                .ok()
                .flatten()
                .map_or(Value::Null, |v| json!(v)),
            Type::FLOAT4 => row
                .try_get::<_, Option<f32>>(index)
                .ok()
                .flatten()
                .map_or(Value::Null, |v| json!(v)),
            Type::FLOAT8 => row
                .try_get::<_, Option<f64>>(index)
                .ok()
                .flatten()
                .map_or(Value::Null, |v| json!(v)),
            _ => row
                .try_get::<_, Option<String>>(index)
                .ok()
                .flatten()
                .map_or(Value::Null, |v| json!(v)),
        }
    }

    /// Convert a row to a JSON object with column names and typed values.
    fn row_to_json(row: &Row) -> Value {
        let mut map = serde_json::Map::new();
        for (i, col) in row.columns().iter().enumerate() {
            let value = Self::column_to_json(row, i, col.type_());
            map.insert(col.name().to_string(), value);
        }
        Value::Object(map)
    }
}

impl Drop for SqlProbe {
    fn drop(&mut self) {
        if let Some(conn) = self.conn.get_mut().take() {
            conn.handle.abort();
        }
    }
}

impl PollingProbe for SqlProbe {
    fn name(&self) -> &str {
        "sql"
    }

    fn poll(&self) -> ProbeFuture<'_> {
        Box::pin(async {
            let mut guard = self.conn.lock().await;

            let needs_reconnect = match &*guard {
                Some(c) => c.handle.is_finished(),
                None => true,
            };

            if needs_reconnect {
                if let Some(old) = guard.take() {
                    old.handle.abort();
                }

                let (client, connection) =
                    tokio_postgres::connect(&self.config.connection_string, NoTls)
                        .await
                        .map_err(|e| ProbeError::Failed(format!("SQL connection failed: {e}")))?;

                let handle = spawn(async move {
                    if let Err(e) = connection.await {
                        warn!(error = %e, "SQL connection error");
                    }
                });

                *guard = Some(SqlConnection { client, handle });
            }

            let conn = guard.as_ref().expect("connection just established");

            let result = conn.client.query(&self.config.query, &[]).await;
            if result.is_err()
                && let Some(old) = guard.take()
            {
                old.handle.abort();
            }
            let rows = result.map_err(|e| ProbeError::Failed(format!("SQL query failed: {e}")))?;

            if rows.is_empty() {
                return Ok(None);
            }

            let json_rows: Vec<Value> = rows.iter().map(Self::row_to_json).collect();
            let data = json!({ "rows": json_rows, "count": rows.len() });

            let hash_input = serde_json::to_vec(&data).unwrap_or_default();
            let content_hash = hex::encode(Sha256::digest(&hash_input));

            Ok(Some(ProbeResult::with_hash(data, content_hash)))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sql_probe_name() {
        let probe = SqlProbe::new(SqlProbeConfig {
            connection_string: "host=localhost".to_string(),
            query: "SELECT 1".to_string(),
        });
        assert_eq!(probe.name(), "sql");
    }

    #[test]
    fn sql_probe_config_clone() {
        let config = SqlProbeConfig {
            connection_string: "host=db dbname=app".to_string(),
            query: "SELECT * FROM events".to_string(),
        };
        let cloned = config.clone();
        assert_eq!(cloned.connection_string, config.connection_string);
        assert_eq!(cloned.query, config.query);
    }
}
