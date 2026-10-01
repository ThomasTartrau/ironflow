#![cfg(all(feature = "store-postgres", feature = "secret-store"))]

//! Integration tests for the PostgreSQL registry of ironflow-auth-proxy.
//!
//! They cover what the in-memory backend cannot: grants surviving a restart,
//! replicas sharing them, the credential encrypted at rest and the token
//! never stored.
//!
//! They need a live database and are ignored by default:
//!
//! ```sh
//! DATABASE_URL=postgres://... cargo test -p ironflow-store \
//!     --features store-postgres,secret-store --test postgres_auth_proxy_grants -- --ignored
//! ```

use std::env::var;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use ironflow_core::auth_proxy::{
    AuthProxyError, AuthProxyRegistry, CredentialKind, IssuedToken, ProxyCredential,
    TokenRejection, TokenRequest, token_id,
};
use ironflow_store::crypto::KeyRing;
use ironflow_store::postgres::PostgresStore;
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row, query};
use uuid::Uuid;

const CREDENTIAL: &str = "sk-ant-oat01-postgres-test";

/// The raw columns of a run's grant.
const RAW_ROW: &str =
    "SELECT id, encrypted_credential, run_id FROM ironflow.auth_proxy_grants WHERE run_id = $1";

/// A run's grant row, every column cast to text.
const ROW_AS_TEXT: &str =
    "SELECT g::text AS row_text FROM ironflow.auth_proxy_grants g WHERE run_id = $1";

fn database_url() -> String {
    var("DATABASE_URL").expect("DATABASE_URL must be set")
}

fn hex_key(byte: u8) -> String {
    format!("{byte:02x}").repeat(32)
}

/// A key ring holding only `byte` repeated, at version 1.
fn ring(byte: u8) -> KeyRing {
    KeyRing::from_spec(&format!("1:{}", hex_key(byte)), None).expect("valid ring")
}

/// A fresh connection pool, as a restarted or another replica would open.
async fn store_with_key(byte: u8) -> PostgresStore {
    let mut store = PostgresStore::new(&database_url())
        .await
        .expect("failed to connect to PostgreSQL");
    store.set_key_ring(ring(byte));
    store
}

fn registry(store: PostgresStore) -> AuthProxyRegistry {
    AuthProxyRegistry::with_backend(Arc::new(store))
}

/// A raw pool, to inspect columns the store does not expose.
async fn raw_pool() -> PgPool {
    PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url())
        .await
        .expect("failed to connect to PostgreSQL")
}

/// A run id unique to this test run: the database is shared across tests.
fn unique_run(label: &str) -> String {
    format!("test-{label}-{}", Uuid::now_v7())
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after the unix epoch")
        .as_secs()
}

fn request(run_id: &str, expires_at: u64) -> TokenRequest {
    TokenRequest {
        run_id: run_id.to_string(),
        step: "review".to_string(),
        expires_at,
        credential: ProxyCredential::new(CredentialKind::OauthToken, CREDENTIAL.to_string()),
    }
}

async fn issue(registry: &AuthProxyRegistry, run_id: &str) -> IssuedToken {
    let now = now();
    registry
        .issue(request(run_id, now + 600), now)
        .await
        .expect("issue")
}

/// Rows of `run_id`, read straight from the table.
async fn count_rows(pool: &PgPool, run_id: &str) -> i64 {
    query("SELECT COUNT(*) AS count FROM ironflow.auth_proxy_grants WHERE run_id = $1")
        .bind(run_id)
        .fetch_one(pool)
        .await
        .expect("count")
        .get("count")
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[tokio::test]
#[ignore = "requires a live PostgreSQL database"]
async fn grant_survives_a_restart() {
    let run_id = unique_run("restart");
    let issued = issue(&registry(store_with_key(0xaa).await), &run_id).await;

    let restarted = registry(store_with_key(0xaa).await);
    let grant = restarted
        .resolve(&issued.token, now())
        .await
        .expect("grant survives");
    assert_eq!(grant.id, issued.id);
    assert_eq!(grant.run_id, run_id);
    assert_eq!(grant.step, "review");
    assert_eq!(grant.credential.kind(), CredentialKind::OauthToken);
    assert_eq!(grant.credential.expose(), CREDENTIAL);

    restarted.revoke_run(&run_id).await.expect("cleanup");
}

#[tokio::test]
#[ignore = "requires a live PostgreSQL database"]
async fn two_replicas_share_revocation() {
    let run_id = unique_run("replicas");
    let a = registry(store_with_key(0xaa).await);
    let b = registry(store_with_key(0xaa).await);
    let first = issue(&a, &run_id).await;
    let second = issue(&a, &run_id).await;
    assert!(b.resolve(&first.token, now()).await.is_ok());

    assert_eq!(b.revoke_run(&run_id).await.expect("revoke run"), 2);
    assert_eq!(
        a.resolve(&first.token, now()).await.unwrap_err(),
        TokenRejection::Unknown
    );
    assert_eq!(
        a.resolve(&second.token, now()).await.unwrap_err(),
        TokenRejection::Unknown
    );
    assert_eq!(a.revoke_run(&run_id).await.expect("revoke run"), 0);
}

#[tokio::test]
#[ignore = "requires a live PostgreSQL database"]
async fn single_revocation_is_seen_by_another_replica() {
    let run_id = unique_run("revoke");
    let a = registry(store_with_key(0xaa).await);
    let b = registry(store_with_key(0xaa).await);
    let issued = issue(&a, &run_id).await;

    assert!(b.revoke(&issued.id).await.expect("revoke"));
    assert!(!a.revoke(&issued.id).await.expect("revoke again"));
    assert_eq!(
        a.resolve(&issued.token, now()).await.unwrap_err(),
        TokenRejection::Unknown
    );
}

#[tokio::test]
#[ignore = "requires a live PostgreSQL database"]
async fn token_is_never_stored_and_credential_is_encrypted() {
    let run_id = unique_run("at-rest");
    let registry = registry(store_with_key(0xaa).await);
    let issued = issue(&registry, &run_id).await;
    let pool = raw_pool().await;

    let row = query(RAW_ROW)
        .bind(&run_id)
        .fetch_one(&pool)
        .await
        .expect("row exists");
    let id: String = row.get("id");
    let encrypted: Vec<u8> = row.get("encrypted_credential");
    let stored_run: String = row.get("run_id");
    assert_eq!(id, token_id(&issued.token));
    assert_ne!(id, issued.token);
    assert_eq!(stored_run, run_id);
    assert!(!contains(&encrypted, CREDENTIAL.as_bytes()));
    assert!(!contains(&encrypted, issued.token.as_bytes()));

    let text: String = query(ROW_AS_TEXT)
        .bind(&run_id)
        .fetch_one(&pool)
        .await
        .expect("row exists")
        .get("row_text");
    assert!(!text.contains(&issued.token), "{text}");
    assert!(!text.contains(CREDENTIAL), "{text}");
    let credential_hex: String = CREDENTIAL.bytes().map(|b| format!("{b:02x}")).collect();
    assert!(!text.contains(&credential_hex), "{text}");

    registry.revoke_run(&run_id).await.expect("cleanup");
}

#[tokio::test]
#[ignore = "requires a live PostgreSQL database"]
async fn purge_expired_deletes_expired_rows() {
    let run_id = unique_run("purge");
    let registry = registry(store_with_key(0xaa).await);
    let pool = raw_pool().await;
    // Two hours back: older than the expired grant of
    // `expired_grant_is_rejected_and_removed_on_resolve`, which runs in
    // parallel and must not be purged from under it.
    let past_now = now() - 7_200;
    registry
        .issue(request(&run_id, past_now + 10), past_now)
        .await
        .expect("issue expired");
    let live = issue(&registry, &run_id).await;
    assert_eq!(count_rows(&pool, &run_id).await, 2);

    assert!(registry.purge_expired(past_now + 10).await.expect("purge") >= 1);
    assert_eq!(count_rows(&pool, &run_id).await, 1);
    assert!(registry.resolve(&live.token, now()).await.is_ok());

    registry.revoke_run(&run_id).await.expect("cleanup");
}

#[tokio::test]
#[ignore = "requires a live PostgreSQL database"]
async fn expired_grant_is_rejected_and_removed_on_resolve() {
    let run_id = unique_run("expired");
    let registry = registry(store_with_key(0xaa).await);
    let pool = raw_pool().await;
    let past_now = now() - 60;
    let issued = registry
        .issue(request(&run_id, past_now + 10), past_now)
        .await
        .expect("issue");

    assert_eq!(
        registry.resolve(&issued.token, now()).await.unwrap_err(),
        TokenRejection::Expired
    );
    assert_eq!(count_rows(&pool, &run_id).await, 0);
    assert_eq!(
        registry.resolve(&issued.token, now()).await.unwrap_err(),
        TokenRejection::Unknown
    );
}

#[tokio::test]
#[ignore = "requires a live PostgreSQL database"]
async fn insert_without_key_ring_fails() {
    let run_id = unique_run("no-key");
    let store = PostgresStore::new(&database_url())
        .await
        .expect("failed to connect to PostgreSQL");
    let registry = registry(store);
    let now = now();

    let result = registry.issue(request(&run_id, now + 600), now).await;
    assert!(
        matches!(result, Err(AuthProxyError::Backend(_))),
        "{result:?}"
    );
    assert_eq!(count_rows(&raw_pool().await, &run_id).await, 0);
}

#[tokio::test]
#[ignore = "requires a live PostgreSQL database"]
async fn wrong_key_ring_cannot_read_grant() {
    let run_id = unique_run("wrong-key");
    let writer = registry(store_with_key(0xaa).await);
    let issued = issue(&writer, &run_id).await;

    let reader = registry(store_with_key(0xbb).await);
    match reader.resolve(&issued.token, now()).await {
        Err(TokenRejection::Unavailable(message)) => {
            assert!(!message.contains(CREDENTIAL), "{message}");
        }
        Ok(grant) => panic!("grant {} decrypted with the wrong key", grant.id),
        Err(other) => panic!("expected Unavailable, got {other:?}"),
    }

    writer.revoke_run(&run_id).await.expect("cleanup");
}
