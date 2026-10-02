//! Shared fixtures for the signal route tests.
//!
//! Every helper here builds real objects: a real `InMemoryStore`, real users,
//! real API keys stored through `ApiKeyStore` and real JWTs minted by
//! `ironflow_auth`.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, from_slice};
use tokio::sync::broadcast;
use tower::ServiceExt;
use uuid::Uuid;

use ironflow_auth::extractor::{API_KEY_PREFIX, API_KEY_SUFFIX_LEN};
use ironflow_auth::jwt::{AccessToken, JwtConfig};
use ironflow_auth::password;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_engine::engine::Engine;
use ironflow_engine::notify::Event;
use ironflow_store::entities::{ApiKeyScope, NewApiKey, NewUser, User};
use ironflow_store::memory::InMemoryStore;
use ironflow_store::store::Store;

use crate::state::AppState;

/// An `AppState` over an in-memory store holding an admin and a member.
pub(super) async fn test_state() -> (AppState, User, User) {
    let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
    let provider = Arc::new(ClaudeCodeProvider::new());
    let engine = Arc::new(Engine::new(store.clone(), provider));
    let jwt_config = Arc::new(JwtConfig {
        secret: "test-secret-for-signals".to_string(),
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

    let admin = create_user(store.as_ref(), "admin", true).await;
    let member = create_user(store.as_ref(), "member", false).await;
    (state, admin, member)
}

async fn create_user(store: &dyn Store, username: &str, is_admin: bool) -> User {
    let password_hash = password::hash("password123").expect("hash");
    store
        .create_user(NewUser {
            email: format!("{username}@example.com"),
            username: username.to_string(),
            password_hash,
            is_admin: Some(is_admin),
        })
        .await
        .expect("create user")
}

/// A `Bearer` JWT header for `user`.
pub(super) fn jwt_header(user: &User, state: &AppState) -> String {
    let token = AccessToken::for_user(user.id, &user.username, user.is_admin, &state.jwt_config)
        .expect("token");
    format!("Bearer {}", token.0)
}

/// A `Bearer` API key header for a key owned by `user` with `scopes`.
pub(super) async fn api_key_header(
    user: &User,
    scopes: Vec<ApiKeyScope>,
    state: &AppState,
) -> String {
    let raw_key = format!(
        "irfl_{}rest-of-secret-key",
        &Uuid::now_v7().simple().to_string()[24..]
    );
    let prefix = &raw_key[..API_KEY_PREFIX.len() + API_KEY_SUFFIX_LEN];
    state
        .store
        .create_api_key(NewApiKey {
            user_id: user.id,
            name: format!("key-{}", Uuid::now_v7()),
            key_hash: password::hash(&raw_key).expect("hash"),
            key_prefix: prefix.to_string(),
            scopes,
            expires_at: None,
            rate_limit_override: None,
        })
        .await
        .expect("create api key");
    format!("Bearer {raw_key}")
}

/// Call `router` with `method` on `uri`, returning the status and JSON body.
pub(super) async fn call(
    router: Router,
    method: &str,
    uri: &str,
    auth: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder().uri(uri).method(method);
    if let Some(auth) = auth {
        builder = builder.header("authorization", auth);
    }
    let req = match body {
        Some(body) => builder
            .header("content-type", "application/json")
            .body(Body::from(body.to_string())),
        None => builder.body(Body::empty()),
    }
    .expect("build request");

    let resp = router.oneshot(req).await.expect("request");
    let status = resp.status();
    let bytes = resp.into_body().collect().await.expect("body").to_bytes();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        from_slice(&bytes).expect("json")
    };
    (status, value)
}
