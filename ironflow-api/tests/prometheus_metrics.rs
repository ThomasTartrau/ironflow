//! Integration tests for the Prometheus `/metrics` endpoint.

#![cfg(feature = "prometheus")]

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use ironflow_auth::extractor::{API_KEY_PREFIX, API_KEY_SUFFIX_LEN};
use ironflow_auth::jwt::{AccessToken, JwtConfig};
use ironflow_auth::password;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_engine::engine::Engine;
use ironflow_engine::notify::Event;
use ironflow_store::entities::{ApiKeyScope, NewApiKey, NewUser};
use ironflow_store::memory::InMemoryStore;
use serde_json::{Value, from_slice};
use tokio::sync::broadcast;
use tower::ServiceExt;
use uuid::Uuid;

use ironflow_api::routes::{RouterConfig, create_router};
use ironflow_api::state::AppState;

fn test_state() -> AppState {
    let store = Arc::new(InMemoryStore::new());
    let provider = Arc::new(ClaudeCodeProvider::new());
    let engine = Arc::new(Engine::new(store.clone(), provider));
    let jwt_config = Arc::new(JwtConfig {
        secret: "test-secret".to_string(),
        access_token_ttl_secs: 900,
        refresh_token_ttl_secs: 604800,
        cookie_domain: None,
        cookie_secure: false,
    });
    let (event_sender, _) = broadcast::channel::<Event>(1);

    AppState::new(
        store,
        engine,
        jwt_config,
        "test-worker-token".to_string(),
        event_sender,
    )
}

/// A `Bearer` header holding the JWT of a fresh user, an admin when
/// `is_admin` is set.
async fn user_jwt(state: &AppState, is_admin: bool) -> String {
    let suffix = Uuid::now_v7().simple().to_string();
    let user = state
        .store
        .create_user(NewUser {
            email: format!("user-{suffix}@example.com"),
            username: format!("user-{suffix}"),
            password_hash: "argon2hash".to_string(),
            is_admin: Some(is_admin),
        })
        .await
        .unwrap();
    let jwt = &state.jwt_config;
    let token = AccessToken::for_user(user.id, &user.username, is_admin, jwt).unwrap();
    format!("Bearer {}", token.0)
}

/// A `Bearer` header holding the JWT of a fresh admin.
async fn admin_jwt(state: &AppState) -> String {
    user_jwt(state, true).await
}

/// A `Bearer` header holding the JWT of a fresh member.
async fn member_jwt(state: &AppState) -> String {
    user_jwt(state, false).await
}

/// A `Bearer` header for an API key holding `scopes`, owned by a fresh user
/// who is an admin when `owner_is_admin` is set.
async fn api_key(state: &AppState, owner_is_admin: bool, scopes: Vec<ApiKeyScope>) -> String {
    let suffix = Uuid::now_v7().simple().to_string();
    let owner = state
        .store
        .create_user(NewUser {
            email: format!("owner-{suffix}@example.com"),
            username: format!("owner-{suffix}"),
            password_hash: "argon2hash".to_string(),
            is_admin: Some(owner_is_admin),
        })
        .await
        .unwrap();
    let raw_key = format!("irfl_{}rest-of-secret-key", &suffix[24..]);
    let prefix = &raw_key[..API_KEY_PREFIX.len() + API_KEY_SUFFIX_LEN];
    state
        .store
        .create_api_key(NewApiKey {
            user_id: owner.id,
            name: format!("key-{suffix}"),
            key_hash: password::hash(&raw_key).unwrap(),
            key_prefix: prefix.to_string(),
            scopes,
            expires_at: None,
            rate_limit_override: None,
        })
        .await
        .unwrap();
    format!("Bearer {raw_key}")
}

#[tokio::test]
async fn metrics_endpoint_returns_prometheus_format() {
    let state = test_state();
    let auth = admin_jwt(&state).await;
    let app = create_router(state, RouterConfig::default());

    let req = Request::builder()
        .uri("/api/v1/metrics")
        .header(AUTHORIZATION, &auth)
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let content_type = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(
        content_type.contains("text/plain"),
        "content-type must be text/plain, got: {content_type}"
    );
}

#[tokio::test]
async fn metrics_endpoint_includes_api_request_metrics() {
    let state = test_state();
    let auth = admin_jwt(&state).await;
    let app = create_router(state, RouterConfig::default());

    // First, make a request to health-check to generate some metrics
    let health_req = Request::builder()
        .uri("/api/v1/health-check")
        .body(Body::empty())
        .unwrap();
    let health_resp = app.clone().oneshot(health_req).await.unwrap();
    assert_eq!(health_resp.status(), StatusCode::OK);

    // Then check that /metrics contains the API request metrics
    let metrics_req = Request::builder()
        .uri("/api/v1/metrics")
        .header(AUTHORIZATION, &auth)
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(metrics_req).await.unwrap();
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let body_str = String::from_utf8_lossy(&body);

    assert!(
        body_str.contains("ironflow_api_requests_total"),
        "metrics must include API request counter"
    );
    assert!(
        body_str.contains("ironflow_api_request_duration_seconds"),
        "metrics must include API request duration histogram"
    );
}

/// Request `/api/v1/metrics`, with `auth` as `Authorization` header when set.
async fn request_metrics(app: Router, auth: Option<&str>) -> (StatusCode, String) {
    let mut builder = Request::builder().uri("/api/v1/metrics");
    if let Some(auth) = auth {
        builder = builder.header(AUTHORIZATION, auth);
    }
    let req = builder.body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&body).into_owned())
}

/// Render `/api/v1/metrics` as text, authenticated with `auth`.
async fn scrape(app: Router, auth: &str) -> String {
    let (status, body) = request_metrics(app, Some(auth)).await;
    assert_eq!(status, StatusCode::OK, "scrape refused: {body}");
    body
}

#[tokio::test]
async fn metrics_endpoint_rejects_an_unauthenticated_scrape() {
    let app = create_router(test_state(), RouterConfig::default());

    let (status, body) = request_metrics(app, None).await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(
        !body.contains("ironflow_"),
        "an unauthenticated caller must not read any metric"
    );
}

#[tokio::test]
async fn metrics_endpoint_rejects_a_member_session() {
    let state = test_state();
    let auth = member_jwt(&state).await;
    let app = create_router(state, RouterConfig::default());

    let (status, _body) = request_metrics(app, Some(&auth)).await;

    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn metrics_endpoint_rejects_a_key_without_stats_read() {
    let state = test_state();
    let auth = api_key(&state, true, vec![ApiKeyScope::RunsRead]).await;
    let app = create_router(state, RouterConfig::default());

    let (status, body) = request_metrics(app, Some(&auth)).await;

    assert_eq!(status, StatusCode::FORBIDDEN);
    let json: Value = from_slice(body.as_bytes()).unwrap();
    assert_eq!(json["error"]["code"], "INSUFFICIENT_SCOPE");
}

#[tokio::test]
async fn metrics_endpoint_rejects_a_stats_read_key_of_a_member() {
    let state = test_state();
    let auth = api_key(&state, false, vec![ApiKeyScope::StatsRead]).await;
    let app = create_router(state, RouterConfig::default());

    let (status, body) = request_metrics(app, Some(&auth)).await;

    assert_eq!(status, StatusCode::FORBIDDEN);
    let json: Value = from_slice(body.as_bytes()).unwrap();
    assert_eq!(json["error"]["code"], "FORBIDDEN");
}

#[tokio::test]
async fn metrics_endpoint_serves_a_stats_read_key_of_an_admin() {
    let state = test_state();
    let auth = api_key(&state, true, vec![ApiKeyScope::StatsRead]).await;
    let app = create_router(state, RouterConfig::default());

    let req = Request::builder()
        .uri("/api/v1/health-check")
        .body(Body::empty())
        .unwrap();
    let _response = app.clone().oneshot(req).await.unwrap();

    let body = scrape(app, &auth).await;
    assert!(
        body.contains("ironflow_api_requests_total"),
        "metrics must include API request counter"
    );
}

#[tokio::test]
async fn metrics_label_requests_by_matched_route_not_raw_url() {
    let state = test_state();
    let auth = admin_jwt(&state).await;
    let app = create_router(state, RouterConfig::default());
    let step_id = Uuid::now_v7();

    let req = Request::builder()
        .method("PUT")
        .uri(format!("/api/v1/internal/steps/{step_id}"))
        .header(AUTHORIZATION, "Bearer test-worker-token")
        .header(CONTENT_TYPE, "application/json")
        .body(Body::from("{}"))
        .unwrap();
    let _response = app.clone().oneshot(req).await.unwrap();

    let body = scrape(app, &auth).await;
    assert!(
        body.contains(r#"path="/api/v1/internal/steps/{id}""#),
        "the route pattern must be the path label"
    );
    assert!(
        !body.contains(&step_id.to_string()),
        "a raw id must never become a label value"
    );
}

#[tokio::test]
async fn metrics_do_not_label_unknown_urls_with_their_raw_path() {
    let state = test_state();
    let auth = admin_jwt(&state).await;
    let app = create_router(state, RouterConfig::default());
    let unknown = format!("/no-such-page-{}", Uuid::now_v7());

    let req = Request::builder()
        .uri(&unknown)
        .body(Body::empty())
        .unwrap();
    let _response = app.clone().oneshot(req).await.unwrap();

    let body = scrape(app, &auth).await;
    assert!(
        !body.contains(&unknown),
        "an unknown URL must not create its own time series"
    );
}

#[tokio::test]
async fn metrics_label_unknown_http_methods_as_other() {
    let state = test_state();
    let auth = admin_jwt(&state).await;
    let app = create_router(state, RouterConfig::default());

    let req = Request::builder()
        .method("FOOBAR")
        .uri("/api/v1/health-check")
        .body(Body::empty())
        .unwrap();
    let _response = app.clone().oneshot(req).await.unwrap();

    let body = scrape(app, &auth).await;
    assert!(
        !body.contains(r#"method="FOOBAR""#),
        "an invented method must not create its own time series"
    );
    assert!(
        body.contains(r#"method="other""#),
        "an invented method must be labelled other"
    );
}

#[tokio::test]
async fn metrics_keep_standard_method_labels() {
    let state = test_state();
    let auth = admin_jwt(&state).await;
    let app = create_router(state, RouterConfig::default());

    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/health-check")
        .body(Body::empty())
        .unwrap();
    let _response = app.clone().oneshot(req).await.unwrap();

    let body = scrape(app, &auth).await;
    assert!(
        body.contains(r#"method="GET""#),
        "a standard method keeps its own label"
    );
}
