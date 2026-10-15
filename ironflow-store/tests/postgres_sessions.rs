#![cfg(feature = "store-postgres")]

//! Integration tests for server-side session revocation on PostgreSQL.
//!
//! They cover the SQL side the in-memory backend cannot: single-use refresh
//! tokens through `DELETE ... RETURNING`, the `token_version` bump and the
//! cascade when a user is deleted.
//!
//! They need a live database and are ignored by default:
//!
//! ```sh
//! DATABASE_URL=postgres://... cargo test -p ironflow-store \
//!     --features store-postgres --test postgres_sessions -- --ignored
//! ```

use std::env::var;

use chrono::{Duration, Utc};
use ironflow_store::api_key_store::ApiKeyStore;
use ironflow_store::entities::{ApiKey, ApiKeyScope, NewApiKey, NewRefreshToken, NewUser, User};
use ironflow_store::error::StoreError;
use ironflow_store::postgres::PostgresStore;
use ironflow_store::user_store::UserStore;
use uuid::Uuid;

fn database_url() -> String {
    var("DATABASE_URL").expect("DATABASE_URL must be set")
}

async fn store() -> PostgresStore {
    PostgresStore::new(&database_url())
        .await
        .expect("failed to connect to PostgreSQL")
}

/// A user with a unique email and username, so tests can share a database.
async fn user(store: &PostgresStore, is_admin: bool) -> User {
    let suffix = Uuid::now_v7().simple().to_string();
    store
        .create_user(NewUser {
            email: format!("session-{suffix}@example.com"),
            username: format!("session-{suffix}"),
            password_hash: "argon2hash".to_string(),
            is_admin: Some(is_admin),
        })
        .await
        .expect("create user")
}

/// Record a refresh token for `user_id` and return its hash.
async fn issue(store: &PostgresStore, user_id: Uuid, ttl: Duration) -> String {
    let token_hash = format!("hash-{}", Uuid::now_v7());
    store
        .store_refresh_token(NewRefreshToken {
            token_hash: token_hash.clone(),
            user_id,
            expires_at: Utc::now() + ttl,
        })
        .await
        .expect("store refresh token");
    token_hash
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn refresh_token_is_single_use() {
    let store = store().await;
    let alice = user(&store, false).await;
    let hash = issue(&store, alice.id, Duration::hours(1)).await;

    assert_eq!(
        store.consume_refresh_token(&hash).await.unwrap(),
        Some(alice.id)
    );
    assert_eq!(store.consume_refresh_token(&hash).await.unwrap(), None);
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn expired_refresh_token_is_rejected() {
    let store = store().await;
    let alice = user(&store, false).await;
    let hash = issue(&store, alice.id, Duration::seconds(-5)).await;

    assert_eq!(store.consume_refresh_token(&hash).await.unwrap(), None);
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn store_refresh_token_for_unknown_user_is_not_found() {
    let store = store().await;
    let err = store
        .store_refresh_token(NewRefreshToken {
            token_hash: format!("hash-{}", Uuid::now_v7()),
            user_id: Uuid::now_v7(),
            expires_at: Utc::now() + Duration::hours(1),
        })
        .await
        .unwrap_err();

    assert!(matches!(err, StoreError::UserNotFound(_)), "got: {err}");
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn revoke_user_sessions_bumps_version_and_drops_refresh_tokens() {
    let store = store().await;
    let alice = user(&store, false).await;
    let bob = user(&store, false).await;
    let alice_hash = issue(&store, alice.id, Duration::hours(1)).await;
    let bob_hash = issue(&store, bob.id, Duration::hours(1)).await;

    assert_eq!(store.revoke_user_sessions(alice.id).await.unwrap(), 1);

    let found = store.find_user_by_id(alice.id).await.unwrap().unwrap();
    assert_eq!(found.token_version, 1);
    assert_eq!(
        store.consume_refresh_token(&alice_hash).await.unwrap(),
        None
    );
    assert_eq!(
        store.consume_refresh_token(&bob_hash).await.unwrap(),
        Some(bob.id)
    );
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn revoke_user_sessions_unknown_user_is_not_found() {
    let store = store().await;
    let err = store
        .revoke_user_sessions(Uuid::now_v7())
        .await
        .unwrap_err();

    assert!(matches!(err, StoreError::UserNotFound(_)), "got: {err}");
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn role_change_bumps_version_and_drops_refresh_tokens() {
    let store = store().await;
    let admin = user(&store, true).await;
    assert_eq!(admin.token_version, 0);
    let hash = issue(&store, admin.id, Duration::hours(1)).await;

    let demoted = store.update_user_role(admin.id, false).await.unwrap();

    assert!(!demoted.is_admin);
    assert_eq!(demoted.token_version, 1);
    assert_eq!(store.consume_refresh_token(&hash).await.unwrap(), None);
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn password_change_bumps_version_and_drops_refresh_tokens() {
    let store = store().await;
    let alice = user(&store, false).await;
    let hash = issue(&store, alice.id, Duration::hours(1)).await;

    store
        .update_user_password(alice.id, "newhash".to_string())
        .await
        .unwrap();

    let found = store.find_user_by_id(alice.id).await.unwrap().unwrap();
    assert_eq!(found.token_version, 1);
    assert_eq!(store.consume_refresh_token(&hash).await.unwrap(), None);
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn deleting_a_user_cascades_to_refresh_tokens() {
    let store = store().await;
    let alice = user(&store, false).await;
    let hash = issue(&store, alice.id, Duration::hours(1)).await;

    store.delete_user(alice.id).await.unwrap();

    assert_eq!(store.consume_refresh_token(&hash).await.unwrap(), None);
}

/// An API key of `user_id` with a unique prefix, so tests can share a database.
async fn api_key(store: &PostgresStore, user_id: Uuid, scopes: Vec<ApiKeyScope>) -> ApiKey {
    let suffix = Uuid::now_v7().simple().to_string();
    store
        .create_api_key(NewApiKey {
            user_id,
            name: format!("key-{suffix}"),
            key_hash: "argon2hash".to_string(),
            key_prefix: format!("t{}", &suffix[suffix.len() - 12..]),
            scopes,
            expires_at: None,
            rate_limit_override: None,
        })
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn demotion_strips_member_forbidden_scopes_from_api_keys() {
    let store = store().await;
    let admin = user(&store, true).await;
    let other = user(&store, true).await;
    let mixed = api_key(
        &store,
        admin.id,
        vec![ApiKeyScope::RunsRead, ApiKeyScope::SignalsSend],
    )
    .await;
    let admin_only = api_key(&store, admin.id, vec![ApiKeyScope::Admin]).await;
    let others = api_key(&store, other.id, vec![ApiKeyScope::Admin]).await;

    store.update_user_role(admin.id, false).await.unwrap();

    let mixed = store.find_api_key_by_id(mixed.id).await.unwrap().unwrap();
    assert_eq!(mixed.scopes, vec![ApiKeyScope::RunsRead]);
    assert!(mixed.is_active);

    let admin_only = store
        .find_api_key_by_id(admin_only.id)
        .await
        .unwrap()
        .unwrap();
    assert!(admin_only.scopes.is_empty());
    assert!(!admin_only.is_active);

    let others = store.find_api_key_by_id(others.id).await.unwrap().unwrap();
    assert_eq!(others.scopes, vec![ApiKeyScope::Admin]);
    assert!(others.is_active);
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn promotion_keeps_api_keys_unchanged() {
    let store = store().await;
    let member = user(&store, false).await;
    let key = api_key(&store, member.id, vec![ApiKeyScope::RunsRead]).await;

    store.update_user_role(member.id, true).await.unwrap();

    let found = store.find_api_key_by_id(key.id).await.unwrap().unwrap();
    assert_eq!(found.scopes, vec![ApiKeyScope::RunsRead]);
    assert!(found.is_active);
}
