#![cfg(all(feature = "store-postgres", feature = "secret-store"))]

//! Integration tests for encryption key rotation on the PostgreSQL store.
//!
//! These exercise the SQL paths the in-memory store cannot cover: the
//! `key_version` column and its backfill, the id-ordered cursor, and the
//! compare-and-swap guard that protects a concurrent write.
//!
//! They need a live database and are ignored by default:
//!
//! ```sh
//! DATABASE_URL=postgres://... cargo test -p ironflow-store \
//!     --features store-postgres,secret-store --test postgres_secret_rotation -- --ignored
//! ```

use ironflow_store::crypto::KeyRing;
use ironflow_store::entities::RotationRequest;
use ironflow_store::postgres::PostgresStore;
use ironflow_store::secret_store::SecretStore;
use sqlx::PgPool;
use sqlx::Row;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

fn database_url() -> String {
    std::env::var("DATABASE_URL").expect("DATABASE_URL must be set")
}

fn hex_key(byte: u8) -> String {
    format!("{byte:02x}").repeat(32)
}

fn two_key_spec() -> String {
    format!("1:{},2:{}", hex_key(0xaa), hex_key(0xbb))
}

/// A store whose ring holds versions 1 and 2, with `active` encrypting.
async fn get_store(active: i32) -> PostgresStore {
    let mut store = PostgresStore::new(&database_url())
        .await
        .expect("failed to connect to PostgreSQL");
    store.set_key_ring(KeyRing::from_spec(&two_key_spec(), Some(active)).expect("valid ring"));
    store
}

/// A raw pool, to inspect columns the store does not expose.
async fn raw_pool() -> PgPool {
    PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url())
        .await
        .expect("failed to connect to PostgreSQL")
}

/// A key unique to this test run: the database is shared across tests.
fn unique_key(label: &str) -> String {
    format!("test/{label}/{}", Uuid::now_v7())
}

/// Read a secret's stored key version straight from the table.
async fn stored_key_version(pool: &PgPool, key: &str) -> i32 {
    sqlx::query("SELECT key_version FROM ironflow.secrets WHERE key = $1")
        .bind(key)
        .fetch_one(pool)
        .await
        .expect("secret exists")
        .get("key_version")
}

/// Read a secret's stored ciphertext straight from the table.
async fn stored_ciphertext(pool: &PgPool, key: &str) -> Vec<u8> {
    sqlx::query("SELECT encrypted_value FROM ironflow.secrets WHERE key = $1")
        .bind(key)
        .fetch_one(pool)
        .await
        .expect("secret exists")
        .get("encrypted_value")
}

/// Rotate only the secrets this test created, in one pass per batch.
async fn rotate_keys(store: &PostgresStore, pool: &PgPool, to_version: i32, keys: &[String]) {
    let mut cursor = None;
    loop {
        let mut request = RotationRequest::new(to_version).with_batch_size(1000);
        request.after_id = cursor;

        let batch = store.rotate_secrets(request).await.expect("rotation runs");
        if batch.is_complete() {
            break;
        }
        cursor = batch.last_id;
    }

    for key in keys {
        assert_eq!(stored_key_version(pool, key).await, to_version);
    }
}

#[tokio::test]
#[ignore = "requires a live PostgreSQL database"]
async fn new_secret_is_written_with_the_active_version() {
    let store = get_store(2).await;
    let pool = raw_pool().await;
    let key = unique_key("active-version");

    store.set_secret(&key, "value").await.expect("set");

    assert_eq!(stored_key_version(&pool, &key).await, 2);

    store.delete_secret(&key).await.expect("cleanup");
}

#[tokio::test]
#[ignore = "requires a live PostgreSQL database"]
async fn secret_written_with_an_older_version_stays_readable() {
    let mut store = get_store(1).await;
    let pool = raw_pool().await;
    let old = unique_key("old");
    store.set_secret(&old, "old-value").await.expect("set");

    store.set_key_ring(KeyRing::from_spec(&two_key_spec(), Some(2)).expect("valid ring"));
    let new = unique_key("new");
    store.set_secret(&new, "new-value").await.expect("set");

    assert_eq!(stored_key_version(&pool, &old).await, 1);
    assert_eq!(stored_key_version(&pool, &new).await, 2);

    assert_eq!(
        store
            .get_secret(&old)
            .await
            .expect("get")
            .expect("found")
            .value,
        "old-value"
    );
    assert_eq!(
        store
            .get_secret(&new)
            .await
            .expect("get")
            .expect("found")
            .value,
        "new-value"
    );

    store.delete_secret(&old).await.expect("cleanup");
    store.delete_secret(&new).await.expect("cleanup");
}

#[tokio::test]
#[ignore = "requires a live PostgreSQL database"]
async fn rotation_re_encrypts_without_changing_identity_or_timestamps() {
    let mut store = get_store(1).await;
    let pool = raw_pool().await;
    let key = unique_key("identity");

    let before = store.set_secret(&key, "value").await.expect("set");
    let ciphertext_before = stored_ciphertext(&pool, &key).await;

    store.set_key_ring(KeyRing::from_spec(&two_key_spec(), Some(2)).expect("valid ring"));
    rotate_keys(&store, &pool, 2, std::slice::from_ref(&key)).await;

    let after = store.get_secret(&key).await.expect("get").expect("found");
    assert_eq!(after.id, before.id);
    assert_eq!(after.key, before.key);
    assert_eq!(after.value, "value");
    assert_eq!(after.created_at, before.created_at);
    assert_eq!(after.updated_at, before.updated_at);

    assert_ne!(stored_ciphertext(&pool, &key).await, ciphertext_before);
    assert_eq!(stored_key_version(&pool, &key).await, 2);

    store.delete_secret(&key).await.expect("cleanup");
}

#[tokio::test]
#[ignore = "requires a live PostgreSQL database"]
async fn rotation_walks_the_stock_batch_by_batch() {
    let mut store = get_store(1).await;
    let pool = raw_pool().await;

    let keys: Vec<String> = (0..5).map(|i| unique_key(&format!("batch-{i}"))).collect();
    for (i, key) in keys.iter().enumerate() {
        store
            .set_secret(key, &format!("value-{i}"))
            .await
            .expect("set");
    }

    store.set_key_ring(KeyRing::from_spec(&two_key_spec(), Some(2)).expect("valid ring"));

    // One secret at a time: every secret stays readable between batches,
    // whichever version it currently sits on.
    //
    // The batch counters are not asserted against the number of keys created
    // here: the table is shared with the other tests in this file, which
    // rotate rows of their own concurrently.
    let mut cursor = None;
    let mut batches = 0;
    loop {
        let mut request = RotationRequest::new(2).with_batch_size(1);
        request.after_id = cursor;

        let batch = store.rotate_secrets(request).await.expect("rotation runs");
        batches += 1;
        assert_eq!(batch.failed, 0);
        assert!(
            batch.rotated <= 1,
            "batch size of 1 must never re-encrypt more than one secret, got {}",
            batch.rotated
        );

        for (i, key) in keys.iter().enumerate() {
            assert_eq!(
                store
                    .get_secret(key)
                    .await
                    .expect("get")
                    .expect("found")
                    .value,
                format!("value-{i}")
            );
        }

        if batch.is_complete() {
            break;
        }
        cursor = batch.last_id;
    }

    assert!(batches >= 1);
    for key in &keys {
        assert_eq!(stored_key_version(&pool, key).await, 2);
        store.delete_secret(key).await.expect("cleanup");
    }
}

#[tokio::test]
#[ignore = "requires a live PostgreSQL database"]
async fn rotation_is_idempotent() {
    let mut store = get_store(1).await;
    let pool = raw_pool().await;
    let key = unique_key("idempotent");
    store.set_secret(&key, "value").await.expect("set");

    store.set_key_ring(KeyRing::from_spec(&two_key_spec(), Some(2)).expect("valid ring"));
    rotate_keys(&store, &pool, 2, std::slice::from_ref(&key)).await;

    let ciphertext = stored_ciphertext(&pool, &key).await;

    // A second pass must leave the already-rotated row untouched.
    rotate_keys(&store, &pool, 2, std::slice::from_ref(&key)).await;
    assert_eq!(stored_ciphertext(&pool, &key).await, ciphertext);

    store.delete_secret(&key).await.expect("cleanup");
}

#[tokio::test]
#[ignore = "requires a live PostgreSQL database"]
async fn concurrent_write_during_rotation_is_not_overwritten() {
    let mut store = get_store(1).await;
    let pool = raw_pool().await;
    let key = unique_key("concurrent");
    store.set_secret(&key, "stale").await.expect("set");

    store.set_key_ring(KeyRing::from_spec(&two_key_spec(), Some(2)).expect("valid ring"));

    // Simulate the write landing between the rotation's SELECT and UPDATE:
    // set_secret moves the row to version 2 with a fresh value, so the
    // rotation's compare-and-swap on key_version = 1 no longer matches.
    store.set_secret(&key, "fresh").await.expect("overwrite");

    rotate_keys(&store, &pool, 2, std::slice::from_ref(&key)).await;

    assert_eq!(
        store
            .get_secret(&key)
            .await
            .expect("get")
            .expect("found")
            .value,
        "fresh"
    );

    store.delete_secret(&key).await.expect("cleanup");
}

#[tokio::test]
#[ignore = "requires a live PostgreSQL database"]
async fn rotating_to_an_unconfigured_version_fails() {
    let store = get_store(2).await;

    let err = store
        .rotate_secrets(RotationRequest::new(9))
        .await
        .expect_err("version 9 is not configured");

    let message = err.to_string();
    assert!(message.contains('9'));
    assert!(message.contains("1, 2"));
}

#[tokio::test]
#[ignore = "requires a live PostgreSQL database"]
async fn key_status_reports_versions_in_use() {
    let mut store = get_store(2).await;
    let key = unique_key("status");
    store.set_secret(&key, "value").await.expect("set");

    let status = store.secret_key_status().await.expect("status");
    assert_eq!(status.active, 2);
    assert_eq!(status.configured, vec![1, 2]);
    assert!(status.in_use.contains(&2));
    assert!(status.is_consistent());

    // Dropping version 2 while a secret still uses it must be visible.
    store.set_key_ring(
        KeyRing::from_spec(&format!("1:{}", hex_key(0xaa)), Some(1)).expect("valid ring"),
    );
    let status = store.secret_key_status().await.expect("status");
    assert!(status.missing.contains(&2));
    assert!(!status.is_consistent());

    store.delete_secret(&key).await.expect("cleanup");
}
