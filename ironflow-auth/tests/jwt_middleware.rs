//! Integration tests for the [`jwt_auth`] middleware against a real store.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::FromRef;
use axum::http::{Request, StatusCode};
use axum::middleware::from_fn_with_state;
use axum::routing::get;
use http_body_util::BodyExt;
use ironflow_auth::jwt::{AccessToken, JwtConfig};
use ironflow_auth::middleware::jwt_auth;
use ironflow_store::entities::NewUser;
use ironflow_store::memory::InMemoryStore;
use ironflow_store::store::Store;
use serde_json::{Value, from_slice};
use tower::ServiceExt;

#[derive(Clone)]
struct TestState {
    jwt_config: Arc<JwtConfig>,
    store: Arc<dyn Store>,
}

impl FromRef<TestState> for Arc<JwtConfig> {
    fn from_ref(state: &TestState) -> Self {
        state.jwt_config.clone()
    }
}

impl FromRef<TestState> for Arc<dyn Store> {
    fn from_ref(state: &TestState) -> Self {
        state.store.clone()
    }
}

fn test_state() -> TestState {
    TestState {
        jwt_config: Arc::new(JwtConfig {
            secret: "test-secret-key-for-middleware-tests".to_string(),
            access_token_ttl_secs: 900,
            refresh_token_ttl_secs: 604800,
            cookie_domain: None,
            cookie_secure: false,
        }),
        store: Arc::new(InMemoryStore::new()),
    }
}

fn protected_app(state: TestState) -> Router {
    Router::new()
        .route("/protected", get(|| async { "protected" }))
        .layer(from_fn_with_state(state.clone(), jwt_auth::<TestState>))
        .with_state(state)
}

#[tokio::test]
async fn rejects_token_of_deleted_user() {
    let state = test_state();
    let user = state
        .store
        .create_user(NewUser {
            email: "deleted@test.com".to_string(),
            username: "deleted".to_string(),
            password_hash: "argon2hash".to_string(),
            is_admin: Some(false),
        })
        .await
        .unwrap();
    let token = AccessToken::for_user(user.id, "deleted", false, &state.jwt_config).unwrap();
    state.store.delete_user(user.id).await.unwrap();

    let app = protected_app(state);

    let req = Request::builder()
        .uri("/protected")
        .header("authorization", format!("Bearer {}", token.0))
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let json: Value = from_slice(&body).unwrap();
    assert_eq!(json["error"]["code"], "TOKEN_REVOKED");
}
