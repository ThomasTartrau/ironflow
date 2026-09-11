//! Integration tests for `GET /api/v1/stats/history`.

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use ironflow_auth::jwt::{AccessToken, JwtConfig};
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_engine::engine::Engine;
use ironflow_engine::notify::Event;
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{NewRun, RunStatus, RunUpdate, TriggerKind};
use ironflow_store::store::Store;
use rust_decimal::Decimal;
use serde_json::{Value as JsonValue, from_slice, json};
use tokio::sync::broadcast;
use tower::ServiceExt;
use uuid::Uuid;

use ironflow_api::routes::{RouterConfig, create_router};
use ironflow_api::state::AppState;

fn test_state(store: Arc<dyn Store>) -> AppState {
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

fn make_auth_header(state: &AppState) -> String {
    let user_id = Uuid::now_v7();
    let token = AccessToken::for_user(user_id, "testuser", false, &state.jwt_config).unwrap();
    format!("Bearer {}", token.0)
}

async fn create_terminal_run(
    store: &dyn Store,
    name: &str,
    status: RunStatus,
    duration_ms: u64,
    cost_usd: Decimal,
) {
    let run = store
        .create_run(NewRun {
            created_by: None,
            workflow_name: name.to_string(),
            trigger: TriggerKind::Manual,
            payload: json!({}),
            max_retries: 0,
            handler_version: None,
            labels: HashMap::new(),
            scheduled_at: None,
            idempotency_key: None,
            max_cost_usd: None,
        })
        .await
        .unwrap()
        .into_run();
    store
        .update_run_status(run.id, RunStatus::Running)
        .await
        .unwrap();
    store.update_run_status(run.id, status).await.unwrap();
    store
        .update_run(
            run.id,
            RunUpdate {
                duration_ms: Some(duration_ms),
                cost_usd: Some(cost_usd),
                ..RunUpdate::default()
            },
        )
        .await
        .unwrap();
}

// Row 1: GET /stats/history returns correct buckets with mixed statuses
#[tokio::test]
async fn returns_buckets_with_mixed_statuses() {
    let store = Arc::new(InMemoryStore::new());

    create_terminal_run(
        store.as_ref(),
        "deploy",
        RunStatus::Completed,
        5000,
        Decimal::new(100, 2),
    )
    .await;
    create_terminal_run(
        store.as_ref(),
        "deploy",
        RunStatus::Failed,
        3000,
        Decimal::new(50, 2),
    )
    .await;
    create_terminal_run(
        store.as_ref(),
        "deploy",
        RunStatus::Cancelled,
        0,
        Decimal::ZERO,
    )
    .await;

    let state = test_state(store);
    let auth_header = make_auth_header(&state);
    let app = create_router(state, RouterConfig::default());

    let req = Request::builder()
        .uri("/api/v1/stats/history?period=24h")
        .header("authorization", auth_header)
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let json_val: JsonValue = from_slice(&body).unwrap();

    assert_eq!(json_val["data"]["period"], "24h");
    assert_eq!(json_val["data"]["granularity"], "1h");

    let buckets = json_val["data"]["buckets"].as_array().unwrap();
    assert!(!buckets.is_empty());

    let total_completed: u64 = buckets
        .iter()
        .map(|b| b["completed"].as_u64().unwrap())
        .sum();
    let total_failed: u64 = buckets.iter().map(|b| b["failed"].as_u64().unwrap()).sum();
    let total_cancelled: u64 = buckets
        .iter()
        .map(|b| b["cancelled"].as_u64().unwrap())
        .sum();

    assert_eq!(total_completed, 1);
    assert_eq!(total_failed, 1);
    assert_eq!(total_cancelled, 1);
}

// Row 2: Workflow filter scopes buckets
#[tokio::test]
async fn filters_by_workflow_name() {
    let store = Arc::new(InMemoryStore::new());

    create_terminal_run(
        store.as_ref(),
        "deploy",
        RunStatus::Completed,
        5000,
        Decimal::new(100, 2),
    )
    .await;
    create_terminal_run(
        store.as_ref(),
        "build",
        RunStatus::Completed,
        3000,
        Decimal::new(50, 2),
    )
    .await;

    let state = test_state(store);
    let auth_header = make_auth_header(&state);
    let app = create_router(state, RouterConfig::default());

    let req = Request::builder()
        .uri("/api/v1/stats/history?period=24h&workflow=deploy")
        .header("authorization", auth_header)
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let json_val: JsonValue = from_slice(&body).unwrap();

    assert_eq!(json_val["data"]["workflow"], "deploy");
    let buckets = json_val["data"]["buckets"].as_array().unwrap();
    let total_completed: u64 = buckets
        .iter()
        .map(|b| b["completed"].as_u64().unwrap())
        .sum();
    assert_eq!(total_completed, 1);
}

// Row 3: Auto granularity defaults
#[tokio::test]
async fn auto_granularity_defaults() {
    let store = Arc::new(InMemoryStore::new());
    let state = test_state(store);
    let auth_header = make_auth_header(&state);

    for (period, expected_gran) in [("24h", "1h"), ("7d", "1d"), ("30d", "1d"), ("90d", "1w")] {
        let app = create_router(state.clone(), RouterConfig::default());
        let req = Request::builder()
            .uri(format!("/api/v1/stats/history?period={period}"))
            .header("authorization", &auth_header)
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json_val: JsonValue = from_slice(&body).unwrap();
        assert_eq!(
            json_val["data"]["granularity"], expected_gran,
            "period={period} should default to granularity={expected_gran}"
        );
    }
}

// Row 4: Explicit granularity overrides auto
#[tokio::test]
async fn explicit_granularity_overrides() {
    let store = Arc::new(InMemoryStore::new());
    let state = test_state(store);
    let auth_header = make_auth_header(&state);
    let app = create_router(state, RouterConfig::default());

    let req = Request::builder()
        .uri("/api/v1/stats/history?period=24h&granularity=1d")
        .header("authorization", auth_header)
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let json_val: JsonValue = from_slice(&body).unwrap();
    assert_eq!(json_val["data"]["granularity"], "1d");
}

// Row 5: Invalid period returns 400
#[tokio::test]
async fn invalid_period_returns_400() {
    let store = Arc::new(InMemoryStore::new());
    let state = test_state(store);
    let auth_header = make_auth_header(&state);
    let app = create_router(state, RouterConfig::default());

    let req = Request::builder()
        .uri("/api/v1/stats/history?period=999d")
        .header("authorization", auth_header)
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// Row 6: Unauthenticated returns 401
#[tokio::test]
async fn unauthenticated_returns_401() {
    let store = Arc::new(InMemoryStore::new());
    let state = test_state(store);
    let app = create_router(state, RouterConfig::default());

    let req = Request::builder()
        .uri("/api/v1/stats/history")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// Row 8: Route registered at /api/v1/stats/history (same as row 7 but explicit check)
// Covered by all tests above -- any 200 proves the route is registered.

// Row 7: Empty store returns empty buckets
#[tokio::test]
async fn empty_store_returns_empty_buckets() {
    let store = Arc::new(InMemoryStore::new());
    let state = test_state(store);
    let auth_header = make_auth_header(&state);
    let app = create_router(state, RouterConfig::default());

    let req = Request::builder()
        .uri("/api/v1/stats/history")
        .header("authorization", auth_header)
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let json_val: JsonValue = from_slice(&body).unwrap();
    let buckets = json_val["data"]["buckets"].as_array().unwrap();
    assert!(buckets.is_empty());
}
