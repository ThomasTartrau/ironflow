//! Integration tests for the Prometheus `/metrics` endpoint.

#![cfg(feature = "prometheus")]

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use ironflow_auth::jwt::JwtConfig;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_engine::engine::Engine;
use ironflow_store::api_key_store::ApiKeyStore;
use ironflow_store::memory::InMemoryStore;
use tower::ServiceExt;

use ironflow_api::routes::{RouterConfig, create_router};
use ironflow_api::state::AppState;

fn test_state() -> AppState {
    let store = Arc::new(InMemoryStore::new());
    let user_store: Arc<dyn ironflow_store::user_store::UserStore> = Arc::new(InMemoryStore::new());
    let api_key_store: Arc<dyn ApiKeyStore> = Arc::new(InMemoryStore::new());
    let provider = Arc::new(ClaudeCodeProvider::new());
    let engine = Arc::new(Engine::new(store.clone(), provider));
    let jwt_config = Arc::new(JwtConfig {
        secret: "test-secret".to_string(),
        access_token_ttl_secs: 900,
        refresh_token_ttl_secs: 604800,
        cookie_domain: None,
        cookie_secure: false,
    });

    AppState::new(
        store,
        user_store,
        api_key_store,
        engine,
        jwt_config,
        "test-worker-token".to_string(),
    )
}

#[tokio::test]
async fn metrics_endpoint_returns_prometheus_format() {
    let state = test_state();
    let app = create_router(state, RouterConfig::default());

    let req = Request::builder()
        .uri("/api/v1/metrics")
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
