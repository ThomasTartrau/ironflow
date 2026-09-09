//! Integration tests for ironflow-ops-postgres.
//!
//! These tests require a running PostgreSQL instance. Set the `DATABASE_URL`
//! environment variable to connect (e.g. `postgres://localhost/ironflow_test`).
//!
//! Run with: `cargo test -p ironflow-ops-postgres --test integration -- --ignored`

use std::sync::Arc;

use ironflow_core::error::OperationError;
use ironflow_core::operation::{NoopSecretResolver, OperationContext};
use sqlx::PgPool;

fn ctx() -> OperationContext {
    OperationContext::new(Arc::new(NoopSecretResolver))
}

async fn pool() -> PgPool {
    let url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for integration tests");
    PgPool::connect(&url).await.expect("failed to connect")
}

mod health_check {
    use ironflow_ops_postgres::admin::health::HealthCheck;

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn returns_healthy() {
        let pool = pool().await;
        let ctx = ctx();
        let op = HealthCheck::new(pool);
        let result = op.run(&ctx).await.unwrap();
        assert!(result.healthy);
    }
}

mod execute_insert {
    use ironflow_ops_postgres::execute::Execute;

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn returns_rows_affected() {
        let pool = pool().await;
        let ctx = ctx();

        sqlx::query("CREATE TABLE IF NOT EXISTS _test_exec (id int)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM _test_exec")
            .execute(&pool)
            .await
            .unwrap();

        let op = Execute::new(
            pool.clone(),
            "INSERT INTO _test_exec VALUES (1), (2)",
            vec![],
        );
        let result = op.run(&ctx).await.unwrap();
        assert_eq!(result.rows_affected, 2);

        sqlx::query("DROP TABLE _test_exec")
            .execute(&pool)
            .await
            .unwrap();
    }
}

mod query_rows {
    use ironflow_ops_postgres::query::QueryRows;

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn returns_json_rows() {
        let pool = pool().await;
        let ctx = ctx();

        sqlx::query("CREATE TABLE IF NOT EXISTS _test_qr (id int, name text)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM _test_qr")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO _test_qr VALUES (1, 'alice'), (2, 'bob')")
            .execute(&pool)
            .await
            .unwrap();

        let op = QueryRows::new(
            pool.clone(),
            "SELECT id, name FROM _test_qr ORDER BY id",
            vec![],
        );
        let result = op.run(&ctx).await.unwrap();
        assert_eq!(result.rows.len(), 2);
        assert_eq!(result.rows[0]["name"], "alice");
        assert_eq!(result.rows[1]["id"], 2);

        sqlx::query("DROP TABLE _test_qr")
            .execute(&pool)
            .await
            .unwrap();
    }
}

mod query_one {
    use ironflow_ops_postgres::query::QueryOne;
    use serde_json::json;

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn returns_single_row() {
        let pool = pool().await;
        let ctx = ctx();

        sqlx::query("CREATE TABLE IF NOT EXISTS _test_qo (id int, name text)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM _test_qo")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO _test_qo VALUES (1, 'alice')")
            .execute(&pool)
            .await
            .unwrap();

        let op = QueryOne::new(
            pool.clone(),
            "SELECT id, name FROM _test_qo WHERE id = $1",
            vec![json!(1)],
        );
        let result = op.run(&ctx).await.unwrap();
        assert_eq!(result.row["name"], "alice");

        sqlx::query("DROP TABLE _test_qo")
            .execute(&pool)
            .await
            .unwrap();
    }
}

mod query_one_not_found {
    use ironflow_ops_postgres::query::QueryOne;

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn fails_on_zero_rows() {
        let pool = pool().await;
        let ctx = ctx();

        let op = QueryOne::new(pool, "SELECT 1 WHERE false", vec![]);
        let err = op.run(&ctx).await.unwrap_err();
        assert!(
            matches!(err, OperationError::External { .. }),
            "expected External, got: {err:?}"
        );
    }
}

mod query_scalar {
    use ironflow_ops_postgres::query::QueryScalar;

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn returns_scalar_value() {
        let pool = pool().await;
        let ctx = ctx();

        let op = QueryScalar::new(pool, "SELECT 42 AS answer", vec![]);
        let result = op.run(&ctx).await.unwrap();
        assert_eq!(result.value, 42);
    }
}

mod execute_batch {
    use ironflow_ops_postgres::execute::ExecuteBatch;

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn executes_multiple_statements() {
        let pool = pool().await;
        let ctx = ctx();

        let op = ExecuteBatch::new(
            pool.clone(),
            vec![
                "CREATE TABLE IF NOT EXISTS _test_eb (id int)".to_string(),
                "INSERT INTO _test_eb VALUES (1)".to_string(),
                "INSERT INTO _test_eb VALUES (2)".to_string(),
            ],
        );
        let result = op.run(&ctx).await.unwrap();
        assert_eq!(result.rows_affected.len(), 3);
        assert_eq!(result.rows_affected[1], 1);
        assert_eq!(result.rows_affected[2], 1);

        sqlx::query("DROP TABLE _test_eb")
            .execute(&pool)
            .await
            .unwrap();
    }
}

mod transaction_commit {
    use ironflow_ops_postgres::execute::Transaction;

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn commits_on_success() {
        let pool = pool().await;
        let ctx = ctx();

        sqlx::query("CREATE TABLE IF NOT EXISTS _test_tx (id int)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM _test_tx")
            .execute(&pool)
            .await
            .unwrap();

        let op = Transaction::new(
            pool.clone(),
            vec![
                "INSERT INTO _test_tx VALUES (1)".to_string(),
                "INSERT INTO _test_tx VALUES (2)".to_string(),
            ],
        );
        let result = op.run(&ctx).await.unwrap();
        assert_eq!(result.rows_affected, vec![1, 1]);

        let count: (i64,) = sqlx::query_as("SELECT count(*) FROM _test_tx")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count.0, 2);

        sqlx::query("DROP TABLE _test_tx")
            .execute(&pool)
            .await
            .unwrap();
    }
}

mod transaction_rollback {
    use ironflow_ops_postgres::execute::Transaction;

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn rolls_back_on_error() {
        let pool = pool().await;
        let ctx = ctx();

        sqlx::query("CREATE TABLE IF NOT EXISTS _test_txr (id int NOT NULL)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM _test_txr")
            .execute(&pool)
            .await
            .unwrap();

        let op = Transaction::new(
            pool.clone(),
            vec![
                "INSERT INTO _test_txr VALUES (1)".to_string(),
                "INSERT INTO _test_txr VALUES (NULL)".to_string(),
            ],
        );
        let err = op.run(&ctx).await;
        assert!(err.is_err(), "expected error on NULL insert");

        let count: (i64,) = sqlx::query_as("SELECT count(*) FROM _test_txr")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count.0, 0, "transaction should have been rolled back");

        sqlx::query("DROP TABLE _test_txr")
            .execute(&pool)
            .await
            .unwrap();
    }
}

mod list_tables {
    use ironflow_ops_postgres::schema::tables::ListTables;

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn returns_tables() {
        let pool = pool().await;
        let ctx = ctx();

        sqlx::query("CREATE TABLE IF NOT EXISTS _test_lt (id int)")
            .execute(&pool)
            .await
            .unwrap();

        let op = ListTables::new(pool.clone(), "public");
        let result = op.run(&ctx).await.unwrap();
        assert!(
            result.tables.contains(&"_test_lt".to_string()),
            "expected _test_lt in {:?}",
            result.tables
        );

        sqlx::query("DROP TABLE _test_lt")
            .execute(&pool)
            .await
            .unwrap();
    }
}

mod list_columns {
    use ironflow_ops_postgres::schema::columns::ListColumns;

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn returns_column_info() {
        let pool = pool().await;
        let ctx = ctx();

        sqlx::query("CREATE TABLE IF NOT EXISTS _test_lc (id int NOT NULL, name text)")
            .execute(&pool)
            .await
            .unwrap();

        let op = ListColumns::new(pool.clone(), "public", "_test_lc");
        let result = op.run(&ctx).await.unwrap();
        assert_eq!(result.columns.len(), 2);
        assert_eq!(result.columns[0].name, "id");
        assert_eq!(result.columns[0].data_type, "integer");
        assert!(!result.columns[0].is_nullable);
        assert_eq!(result.columns[1].name, "name");
        assert!(result.columns[1].is_nullable);

        sqlx::query("DROP TABLE _test_lc")
            .execute(&pool)
            .await
            .unwrap();
    }
}

mod table_exists {
    use ironflow_ops_postgres::schema::tables::TableExists;

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn returns_true_for_existing() {
        let pool = pool().await;
        let ctx = ctx();

        sqlx::query("CREATE TABLE IF NOT EXISTS _test_te (id int)")
            .execute(&pool)
            .await
            .unwrap();

        let op = TableExists::new(pool.clone(), "public", "_test_te");
        let result = op.run(&ctx).await.unwrap();
        assert!(result.exists);

        sqlx::query("DROP TABLE _test_te")
            .execute(&pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    #[ignore]
    async fn returns_false_for_missing() {
        let pool = pool().await;
        let ctx = ctx();

        let op = TableExists::new(pool, "public", "_nonexistent_table_xyz");
        let result = op.run(&ctx).await.unwrap();
        assert!(!result.exists);
    }
}

mod database_size {
    use ironflow_ops_postgres::admin::size::DatabaseSize;

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn returns_positive_size() {
        let pool = pool().await;
        let ctx = ctx();

        let db_name: (String,) = sqlx::query_as("SELECT current_database()")
            .fetch_one(&pool)
            .await
            .unwrap();

        let op = DatabaseSize::new(pool, &db_name.0);
        let result = op.run(&ctx).await.unwrap();
        assert!(result.size_bytes > 0, "expected positive size");
    }
}

mod active_connections {
    use ironflow_ops_postgres::admin::connections::ActiveConnections;

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn returns_at_least_one() {
        let pool = pool().await;
        let ctx = ctx();

        let op = ActiveConnections::new(pool);
        let result = op.run(&ctx).await.unwrap();
        assert!(result.count >= 1, "expected at least 1 connection");
    }
}

mod invalid_sql {
    use ironflow_ops_postgres::query::QueryRows;

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn returns_operation_error() {
        let pool = pool().await;
        let ctx = ctx();

        let op = QueryRows::new(pool, "SELECT * FROM _nonexistent_table_xyz", vec![]);
        let err = op.run(&ctx).await.unwrap_err();
        assert!(
            matches!(err, OperationError::External { .. }),
            "expected External error, got: {err:?}"
        );
    }
}
