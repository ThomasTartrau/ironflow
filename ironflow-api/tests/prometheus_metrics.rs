//! Integration tests for the Prometheus `/metrics` endpoint.

#![cfg(feature = "prometheus")]

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use ironflow_auth::jwt::JwtConfig;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_engine::engine::Engine;
use ironflow_engine::notify::Event;
use ironflow_store::memory::InMemoryStore;
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

/// Render `/api/v1/metrics` as text.
async fn scrape(app: Router) -> String {
    let req = Request::builder()
        .uri("/api/v1/metrics")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8_lossy(&body).into_owned()
}

#[tokio::test]
async fn metrics_label_requests_by_matched_route_not_raw_url() {
    let app = create_router(test_state(), RouterConfig::default());
    let step_id = Uuid::now_v7();

    let req = Request::builder()
        .method("PUT")
        .uri(format!("/api/v1/internal/steps/{step_id}"))
        .header(AUTHORIZATION, "Bearer test-worker-token")
        .header(CONTENT_TYPE, "application/json")
        .body(Body::from("{}"))
        .unwrap();
    let _response = app.clone().oneshot(req).await.unwrap();

    let body = scrape(app).await;
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
    let app = create_router(test_state(), RouterConfig::default());
    let unknown = format!("/no-such-page-{}", Uuid::now_v7());

    let req = Request::builder()
        .uri(&unknown)
        .body(Body::empty())
        .unwrap();
    let _response = app.clone().oneshot(req).await.unwrap();

    let body = scrape(app).await;
    assert!(
        !body.contains(&unknown),
        "an unknown URL must not create its own time series"
    );
}
