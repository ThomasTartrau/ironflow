//! Tests for automatic retry, rate-limiting, and backoff behaviour.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use ironflow_sdk::ClientBuilder;
use ironflow_sdk::client::ClientConfig;
use ironflow_sdk::retry::RetryConfig;
use tokio::net::TcpListener;

#[derive(Clone)]
struct Counter(Arc<AtomicU32>);

impl Counter {
    fn new() -> Self {
        Self(Arc::new(AtomicU32::new(0)))
    }

    fn inc(&self) -> u32 {
        self.0.fetch_add(1, Ordering::SeqCst)
    }

    fn get(&self) -> u32 {
        self.0.load(Ordering::SeqCst)
    }
}

async fn handler_503_then_200(State(counter): State<Counter>) -> impl IntoResponse {
    let attempt = counter.inc();
    if attempt < 2 {
        (StatusCode::SERVICE_UNAVAILABLE, "unavailable").into_response()
    } else {
        let body = serde_json::json!({ "data": [], "meta": {} });
        (StatusCode::OK, axum::Json(body)).into_response()
    }
}

async fn handler_always_503(State(counter): State<Counter>) -> impl IntoResponse {
    counter.inc();
    (StatusCode::SERVICE_UNAVAILABLE, "unavailable")
}

async fn handler_429_with_retry_after(State(counter): State<Counter>) -> impl IntoResponse {
    let attempt = counter.inc();
    if attempt < 1 {
        (
            StatusCode::TOO_MANY_REQUESTS,
            [("retry-after", "1")],
            axum::Json(serde_json::json!({
                "error": { "code": "RATE_LIMIT_EXCEEDED", "message": "slow down" }
            })),
        )
            .into_response()
    } else {
        let body = serde_json::json!({ "data": [], "meta": {} });
        (StatusCode::OK, axum::Json(body)).into_response()
    }
}

async fn handler_400(State(counter): State<Counter>) -> impl IntoResponse {
    counter.inc();
    (
        StatusCode::BAD_REQUEST,
        axum::Json(serde_json::json!({
            "error": { "code": "BAD_REQUEST", "message": "invalid" }
        })),
    )
}

fn make_client_with_retries(base_url: &str, max_retries: u32) -> ironflow_sdk::IronflowClient {
    let config = ClientConfig {
        base_url: base_url.to_string(),
        api_key: "test-key".to_string(),
        timeout: Duration::from_secs(5),
    };
    let retry = RetryConfig {
        max_retries,
        base_delay: Duration::from_millis(10),
        max_delay: Duration::from_millis(100),
    };
    ironflow_sdk::IronflowClient::from_config_with_retry(
        config,
        retry,
        ironflow_sdk::rate_limit::RateLimiter::new(),
    )
}

// ── Row 1: retry succeeds after transient errors ─────────────────

#[tokio::test]
async fn retry_succeeds_after_transient_503() {
    let counter = Counter::new();
    let app = Router::new()
        .route("/api/v1/runs", get(handler_503_then_200))
        .with_state(counter.clone());

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = make_client_with_retries(&format!("http://{addr}"), 3);
    let result = client.list_runs().await;

    assert!(result.is_ok(), "should succeed after retries: {result:?}");
    assert_eq!(counter.get(), 3);
}

// ── Row 2: retry respects Retry-After header ─────────────────────

#[tokio::test]
async fn retry_respects_retry_after_header() {
    let counter = Counter::new();
    let app = Router::new()
        .route("/api/v1/runs", get(handler_429_with_retry_after))
        .with_state(counter.clone());

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = make_client_with_retries(&format!("http://{addr}"), 3);
    let start = tokio::time::Instant::now();
    let result = client.list_runs().await;

    assert!(result.is_ok());
    assert!(start.elapsed() >= Duration::from_secs(1));
    assert_eq!(counter.get(), 2);
}

// ── Row 3: no retry on 4xx (except 429) ──────────────────────────

#[tokio::test]
async fn no_retry_on_client_error_400() {
    let counter = Counter::new();
    let app = Router::new()
        .route("/api/v1/runs", get(handler_400))
        .with_state(counter.clone());

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = make_client_with_retries(&format!("http://{addr}"), 3);
    let err = client.list_runs().await.unwrap_err();

    assert_eq!(err.status(), Some(400));
    assert_eq!(counter.get(), 1, "should NOT have retried on 400");
}

// ── Row 4: exhausted after max retries ───────────────────────────

#[tokio::test]
async fn exhausted_after_max_retries() {
    let counter = Counter::new();
    let app = Router::new()
        .route("/api/v1/runs", get(handler_always_503))
        .with_state(counter.clone());

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = make_client_with_retries(&format!("http://{addr}"), 2);
    let err = client.list_runs().await.unwrap_err();

    assert!(
        err.is_exhausted(),
        "all retries exhausted should return Exhausted error, got: {err}"
    );
    assert_eq!(err.status(), Some(503));
    assert_eq!(counter.get(), 3, "1 initial + 2 retries = 3 total");
}

// ── Row 5: no retry when disabled (max_retries=0) ────────────────

#[tokio::test]
async fn no_retry_when_disabled() {
    let counter = Counter::new();
    let app = Router::new()
        .route("/api/v1/runs", get(handler_503_then_200))
        .with_state(counter.clone());

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = make_client_with_retries(&format!("http://{addr}"), 0);
    let err = client.list_runs().await.unwrap_err();

    assert_eq!(err.status(), Some(503));
    assert_eq!(counter.get(), 1, "should NOT have retried");
}

// ── Row 11: rate limiter delays after 429 ────────────────────────

#[tokio::test]
async fn rate_limiter_delays_after_429() {
    use ironflow_sdk::rate_limit::RateLimiter;

    let limiter = RateLimiter::new();
    limiter.record(Duration::from_secs(1)).await;

    let counter = Counter::new();
    let app = Router::new()
        .route(
            "/api/v1/runs",
            get(|State(c): State<Counter>| async move {
                c.inc();
                (
                    StatusCode::OK,
                    axum::Json(serde_json::json!({ "data": [], "meta": {} })),
                )
            }),
        )
        .with_state(counter.clone());

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = ironflow_sdk::IronflowClient::from_config_with_retry(
        ClientConfig {
            base_url: format!("http://{addr}"),
            api_key: "test-key".to_string(),
            timeout: Duration::from_secs(5),
        },
        RetryConfig {
            max_retries: 0,
            base_delay: Duration::from_millis(10),
            max_delay: Duration::from_millis(100),
        },
        limiter,
    );

    let start = tokio::time::Instant::now();
    let _ = client.list_runs().await.unwrap();
    assert!(
        start.elapsed() >= Duration::from_millis(700),
        "rate limiter should have delayed the request, got {:?}",
        start.elapsed()
    );
}

// ── Row 12: rate limiter disabled does not block ─────────────────

#[tokio::test]
async fn rate_limiter_disabled_does_not_block() {
    let counter = Counter::new();
    let app = Router::new()
        .route(
            "/api/v1/runs",
            get(|State(c): State<Counter>| async move {
                c.inc();
                (
                    StatusCode::OK,
                    axum::Json(serde_json::json!({ "data": [], "meta": {} })),
                )
            }),
        )
        .with_state(counter.clone());

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = ClientBuilder::new(&format!("http://{addr}"), "test-key")
        .with_max_retries(0)
        .with_rate_limit(false)
        .with_timeout(Duration::from_secs(5))
        .build();

    let start = tokio::time::Instant::now();
    let result = client.list_runs().await;
    assert!(result.is_ok());
    assert!(start.elapsed() < Duration::from_secs(1));
}
