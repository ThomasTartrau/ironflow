//! Shared fixtures for the approval-delegation route tests.
//!
//! Every helper here builds real objects: a real `InMemoryStore`, real users
//! created through `UserStore`, and real JWTs minted by `ironflow_auth`.

use std::sync::Arc;

use tokio::sync::broadcast;
use uuid::Uuid;

use ironflow_auth::jwt::{AccessToken, JwtConfig};
use ironflow_auth::password;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_engine::engine::Engine;
use ironflow_engine::notify::Event;
use ironflow_store::entities::{NewUser, User};
use ironflow_store::memory::InMemoryStore;
use ironflow_store::store::Store;

use crate::state::AppState;

/// The three users every delegation test works with.
pub(super) struct Users {
    /// The delegator.
    pub(super) alice: User,
    /// The delegate.
    pub(super) bob: User,
    /// An unrelated third party.
    pub(super) carol: User,
}

/// An `AppState` over an in-memory store holding `alice`, `bob` and `carol`.
pub(super) async fn test_state() -> (AppState, Users) {
    let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
    let provider = Arc::new(ClaudeCodeProvider::new());
    let engine = Arc::new(Engine::new(store.clone(), provider));
    let jwt_config = Arc::new(JwtConfig {
        secret: "test-secret-for-approval-delegations".to_string(),
        access_token_ttl_secs: 900,
        refresh_token_ttl_secs: 604800,
        cookie_domain: None,
        cookie_secure: false,
    });
    let (event_sender, _) = broadcast::channel::<Event>(1);
    let state = AppState::new(
        store.clone(),
        engine,
        jwt_config,
        "test-worker-token".to_string(),
        event_sender,
    );

    let alice = create_user(store.as_ref(), "alice").await;
    let bob = create_user(store.as_ref(), "bob").await;
    let carol = create_user(store.as_ref(), "carol").await;

    (state, Users { alice, bob, carol })
}

async fn create_user(store: &dyn Store, username: &str) -> User {
    let password_hash = password::hash("password123").expect("hash");
    store
        .create_user(NewUser {
            email: format!("{username}@example.com"),
            username: username.to_string(),
            password_hash,
            // The first user created would otherwise become an implicit admin.
            is_admin: Some(false),
        })
        .await
        .expect("create user")
}

/// A `Bearer` header for a non-admin user.
pub(super) fn member_header(user: &User, state: &AppState) -> String {
    bearer(user.id, &user.username, false, state)
}

/// A `Bearer` header for an admin session bound to `user`.
pub(super) fn admin_header(user: &User, state: &AppState) -> String {
    bearer(user.id, &user.username, true, state)
}

fn bearer(user_id: Uuid, username: &str, is_admin: bool, state: &AppState) -> String {
    let token =
        AccessToken::for_user(user_id, username, is_admin, &state.jwt_config).expect("token");
    format!("Bearer {}", token.0)
}
