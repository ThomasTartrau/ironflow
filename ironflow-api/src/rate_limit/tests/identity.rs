//! Bucket selection by caller identity, and the rate-limit headers.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use ironflow_store::memory::InMemoryStore;
use serde_json::{Value as JsonValue, from_slice};
use tower::ServiceExt;

use super::*;

fn remaining(resp: &Response) -> u32 {
    resp.headers()
        .get("x-ratelimit-remaining")
        .unwrap()
        .to_str()
        .unwrap()
        .parse()
        .unwrap()
}

#[tokio::test]
async fn unauthenticated_uses_ip_bucket() {
    let ctx = test_ctx(2);
    let app = test_app(ctx.clone());

    let resp = app.oneshot(ip_request("1.2.3.4")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let app = test_app(ctx.clone());
    let resp = app.oneshot(ip_request("1.2.3.4")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let app = test_app(ctx);
    let resp = app.oneshot(ip_request("1.2.3.4")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn api_key_auth_uses_separate_bucket() {
    let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
    let (_api_key_id, raw_key) = setup_api_key_in_store(&store, None).await;
    let ctx = ctx_with_store(store, 2);

    // Use up both IP requests from 1.2.3.4
    let app = test_app(ctx.clone());
    let _ = app.oneshot(ip_request("1.2.3.4")).await;
    let app = test_app(ctx.clone());
    let _ = app.oneshot(ip_request("1.2.3.4")).await;

    // IP 1.2.3.4 is now exhausted
    let app = test_app(ctx.clone());
    let resp = app.oneshot(ip_request("1.2.3.4")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);

    // But API key from same IP still works (separate bucket)
    let app = test_app(ctx);
    let req = header_request("1.2.3.4", "authorization", &format!("Bearer {raw_key}"));
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn jwt_auth_uses_user_bucket() {
    let ctx = test_ctx(2);
    let user_id = Uuid::now_v7();
    let token = AccessToken::for_user(user_id, "alice", false, &ctx.jwt_config).unwrap();

    // Use up both IP requests
    let app = test_app(ctx.clone());
    let _ = app.oneshot(ip_request("10.0.0.1")).await;
    let app = test_app(ctx.clone());
    let _ = app.oneshot(ip_request("10.0.0.1")).await;

    // IP exhausted
    let app = test_app(ctx.clone());
    let resp = app.oneshot(ip_request("10.0.0.1")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);

    // JWT from same IP still works (separate User bucket)
    let app = test_app(ctx);
    let req = header_request("10.0.0.1", "authorization", &format!("Bearer {}", token.0));
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn jwt_cookie_uses_user_bucket() {
    let ctx = test_ctx(1);
    let user_id = Uuid::now_v7();
    let token = AccessToken::for_user(user_id, "cookie-user", false, &ctx.jwt_config).unwrap();

    // Exhaust IP bucket
    let app = test_app(ctx.clone());
    let _ = app.oneshot(ip_request("20.0.0.1")).await;

    // IP exhausted
    let app = test_app(ctx.clone());
    let resp = app.oneshot(ip_request("20.0.0.1")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);

    // JWT in cookie from same IP still works (separate User bucket)
    let app = test_app(ctx);
    let req = header_request(
        "20.0.0.1",
        "cookie",
        &format!("ironflow_session={}", token.0),
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn invalid_bearer_falls_back_to_ip() {
    let ctx = test_ctx(1);

    // First request with garbage Bearer uses IP bucket
    let app = test_app(ctx.clone());
    let req = header_request("30.0.0.1", "authorization", "Bearer not_a_valid_token");
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Second request from same IP is rate limited (same bucket)
    let app = test_app(ctx);
    let req = header_request("30.0.0.1", "authorization", "Bearer not_a_valid_token");
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn remaining_header_decrements() {
    let ctx = test_ctx(3);

    let app = test_app(ctx.clone());
    let resp = app.oneshot(ip_request("2.2.2.2")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(remaining(&resp), 2);

    let app = test_app(ctx.clone());
    let resp = app.oneshot(ip_request("2.2.2.2")).await.unwrap();
    assert_eq!(remaining(&resp), 1);

    let app = test_app(ctx);
    let resp = app.oneshot(ip_request("2.2.2.2")).await.unwrap();
    assert_eq!(remaining(&resp), 0);
}

#[tokio::test]
async fn reset_header_present() {
    let ctx = test_ctx(5);
    let app = test_app(ctx);

    let resp = app.oneshot(ip_request("3.3.3.3")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let reset: u64 = resp
        .headers()
        .get("x-ratelimit-reset")
        .unwrap()
        .to_str()
        .unwrap()
        .parse()
        .unwrap();

    let now = now_epoch();
    assert!(
        reset > now,
        "reset {reset} should be in the future (now {now})"
    );
    assert!(
        reset <= now + WINDOW_SECS,
        "reset {reset} should be within one window of now {now}"
    );
}

#[tokio::test]
async fn api_key_override_uses_custom_limit() {
    let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
    let (_api_key_id, raw_key) = setup_api_key_in_store(&store, Some(1)).await;
    let ctx = ctx_with_store(store, 100);

    let app = test_app(ctx.clone());
    let req = Request::builder()
        .uri("/test")
        .header("authorization", format!("Bearer {raw_key}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers()
            .get("x-ratelimit-limit")
            .unwrap()
            .to_str()
            .unwrap(),
        "1"
    );

    // Second request with override=1 should be rejected
    let app = test_app(ctx);
    let req = Request::builder()
        .uri("/test")
        .header("authorization", format!("Bearer {raw_key}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn api_key_override_zero_disables_rate_limiting() {
    let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
    let (_api_key_id, raw_key) = setup_api_key_in_store(&store, Some(0)).await;
    let ctx = ctx_with_store(store, 1);

    // override=0 means unlimited: many requests should all pass
    for _ in 0..10 {
        let app = test_app(ctx.clone());
        let req = Request::builder()
            .uri("/test")
            .header("authorization", format!("Bearer {raw_key}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
}

#[tokio::test]
async fn rate_limited_response_includes_all_headers() {
    let ctx = test_ctx(1);

    let app = test_app(ctx.clone());
    let _ = app.oneshot(ip_request("5.5.5.5")).await;

    let app = test_app(ctx);
    let resp = app.oneshot(ip_request("5.5.5.5")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);

    assert!(resp.headers().contains_key("retry-after"));
    assert!(resp.headers().contains_key("x-ratelimit-limit"));
    assert!(resp.headers().contains_key("x-ratelimit-remaining"));
    assert!(resp.headers().contains_key("x-ratelimit-reset"));
    assert_eq!(remaining(&resp), 0);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let json_val: JsonValue = from_slice(&body).unwrap();
    assert_eq!(json_val["error"]["code"], "RATE_LIMIT_EXCEEDED");
}

#[tokio::test]
async fn different_ips_have_separate_limits() {
    let ctx = test_ctx(1);

    let app = test_app(ctx.clone());
    let resp = app.oneshot(ip_request("10.0.0.1")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let app = test_app(ctx);
    let resp = app.oneshot(ip_request("10.0.0.2")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn allows_requests_within_limit() {
    let ctx = test_ctx(5);
    let app = test_app(ctx);

    let resp = app.oneshot(ip_request("1.2.3.4")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(resp.headers().contains_key("x-ratelimit-limit"));
}

#[tokio::test]
async fn rejects_when_limit_exceeded() {
    let ctx = test_ctx(2);

    let app = test_app(ctx.clone());
    let _ = app.oneshot(ip_request("1.2.3.4")).await;
    let app = test_app(ctx.clone());
    let _ = app.oneshot(ip_request("1.2.3.4")).await;

    let app = test_app(ctx);
    let resp = app.oneshot(ip_request("1.2.3.4")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let json_val: JsonValue = from_slice(&body).unwrap();
    assert_eq!(json_val["error"]["code"], "RATE_LIMIT_EXCEEDED");
}

#[tokio::test]
async fn includes_retry_after_header() {
    let ctx = test_ctx(1);

    let app = test_app(ctx.clone());
    let _ = app.oneshot(ip_request("1.2.3.4")).await;

    let app = test_app(ctx);
    let resp = app.oneshot(ip_request("1.2.3.4")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(resp.headers().contains_key("retry-after"));
}

#[tokio::test]
async fn general_limiter_allows_more_requests() {
    let ctx = test_ctx(60);

    for _ in 0..10 {
        let app = test_app(ctx.clone());
        let resp = app.oneshot(ip_request("1.2.3.4")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
