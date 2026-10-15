//! Bucket selection by caller identity, and the rate-limit headers.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::Duration;
use http_body_util::BodyExt;
use ironflow_store::entities::ApiKeyUpdate;
use ironflow_store::memory::InMemoryStore;
use serde_json::{Value as JsonValue, from_slice};
use tokio::task::JoinSet;
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
    let (_api_key_id, raw_key) = setup_api_key_in_store(&store, None, false).await;
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
async fn dashboard_cookie_session_is_counted_per_user() {
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

    // JWT in the dashboard cookie from same IP still works (separate User bucket)
    let app = test_app(ctx);
    let req = header_request(
        "20.0.0.1",
        "cookie",
        &format!("theme=dark; {AUTH_COOKIE_NAME}={}", token.0),
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn legacy_ironflow_session_cookie_is_not_an_identity() {
    let ctx = test_ctx(1);
    let user_id = Uuid::now_v7();
    let token = AccessToken::for_user(user_id, "cookie-user", false, &ctx.jwt_config).unwrap();

    let resp = test_app(ctx.clone())
        .oneshot(ip_request("20.0.0.2"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let cookie = format!("ironflow_session={}", token.0);
    let req = header_request("20.0.0.2", "cookie", &cookie);
    let resp = test_app(ctx).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn api_key_prefix_with_wrong_secret_uses_ip_bucket() {
    let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
    setup_api_key_in_store(&store, Some(0), true).await;
    let ctx = ctx_with_store(store, 1);

    let resp = test_app(ctx.clone())
        .oneshot(ip_request("40.0.0.1"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let req = header_request("40.0.0.1", "authorization", "Bearer irfl_abcdef12WRONG");
    let resp = test_app(ctx).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn forged_secret_does_not_inherit_unlimited_override() {
    let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
    setup_api_key_in_store(&store, Some(0), true).await;
    let ctx = ctx_with_store(store, 1);

    for expected in [StatusCode::OK, StatusCode::TOO_MANY_REQUESTS] {
        let req = header_request("40.0.0.2", "authorization", "Bearer irfl_abcdef12WRONG");
        let resp = test_app(ctx.clone()).oneshot(req).await.unwrap();
        assert_eq!(resp.status(), expected);
    }
}

#[tokio::test]
async fn api_key_secret_guessing_is_throttled_before_argon2() {
    let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
    setup_api_key_in_store(&store, None, true).await;
    let ctx = ctx_with_store(store, 60);

    for i in 0..KEY_CHECKS_PER_MINUTE {
        let bearer = format!("Bearer irfl_abcdef12WRONG{i}");
        let req = header_request("60.0.0.1", "authorization", &bearer);
        let resp = test_app(ctx.clone()).oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "guess {i}");
    }

    let req = header_request("60.0.0.1", "authorization", "Bearer irfl_abcdef12WRONGX");
    let resp = test_app(ctx.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let json_val: JsonValue = from_slice(&body).unwrap();
    assert_eq!(json_val["error"]["code"], "RATE_LIMIT_EXCEEDED");

    // The IP bucket still has room: the refusal came from the verification
    // budget.
    let resp = test_app(ctx.clone())
        .oneshot(ip_request("60.0.0.1"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // The budget is per client.
    let req = header_request("60.0.0.2", "authorization", "Bearer irfl_abcdef12WRONGY");
    let resp = test_app(ctx).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn expired_api_key_uses_ip_bucket() {
    let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
    let expired = Utc::now() - Duration::hours(1);
    let (_api_key_id, raw_key) =
        setup_expiring_api_key_in_store(&store, Some(0), true, Some(expired)).await;
    let ctx = ctx_with_store(store, 1);

    let resp = test_app(ctx.clone())
        .oneshot(ip_request("40.0.0.3"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let req = header_request("40.0.0.3", "authorization", &format!("Bearer {raw_key}"));
    let resp = test_app(ctx).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn inactive_api_key_uses_ip_bucket() {
    let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
    let (api_key_id, raw_key) = setup_api_key_in_store(&store, Some(0), true).await;
    let deactivate = ApiKeyUpdate {
        is_active: Some(false),
        ..ApiKeyUpdate::default()
    };
    store.update_api_key(api_key_id, deactivate).await.unwrap();
    let ctx = ctx_with_store(store, 1);

    let resp = test_app(ctx.clone())
        .oneshot(ip_request("40.0.0.4"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let req = header_request("40.0.0.4", "authorization", &format!("Bearer {raw_key}"));
    let resp = test_app(ctx).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn member_owned_override_is_ignored() {
    let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
    let (_api_key_id, raw_key) = setup_api_key_in_store(&store, Some(0), false).await;
    let ctx = ctx_with_store(store, 1);
    let bearer = format!("Bearer {raw_key}");

    let req = header_request("40.0.0.5", "authorization", &bearer);
    let resp = test_app(ctx.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let req = header_request("40.0.0.5", "authorization", &bearer);
    let resp = test_app(ctx).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(resp.headers()["x-ratelimit-limit"], "1");
}

#[tokio::test]
async fn verified_api_key_is_cached() {
    let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
    let (_api_key_id, raw_key) = setup_api_key_in_store(&store, None, false).await;
    let ctx = ctx_with_store(store, 2);
    let bearer = format!("Bearer {raw_key}");

    let req = header_request("40.0.0.6", "authorization", &bearer);
    let resp = test_app(ctx.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(ctx.limiter.verified_keys.len(), 1);

    // Exhaust the IP bucket: the cached key keeps its own bucket.
    for _ in 0..2 {
        let req = ip_request("40.0.0.6");
        let _ = test_app(ctx.clone()).oneshot(req).await;
    }
    let req = header_request("40.0.0.6", "authorization", &bearer);
    let resp = test_app(ctx.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(remaining(&resp), 0);
    assert_eq!(ctx.limiter.verified_keys.len(), 1);
}

#[tokio::test]
async fn replayed_wrong_secret_is_cached_and_spends_no_verification_budget() {
    let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
    setup_api_key_in_store(&store, None, true).await;
    let ctx = ctx_with_store(store, 60);

    // The verdict of the first attempt is cached, so the replays cost no
    // Argon2 and never exhaust the verification budget.
    for i in 0..KEY_CHECKS_PER_MINUTE * 2 {
        let req = header_request("60.0.0.3", "authorization", "Bearer irfl_abcdef12WRONG");
        let resp = test_app(ctx.clone()).oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "replay {i}");
    }
    assert_eq!(ctx.limiter.verified_keys.len(), 1);
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
    let (_api_key_id, raw_key) = setup_api_key_in_store(&store, Some(1), true).await;
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
    let (_api_key_id, raw_key) = setup_api_key_in_store(&store, Some(0), true).await;
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

#[tokio::test]
async fn unknown_api_key_prefix_is_not_cached() {
    let ctx = test_ctx(60);

    // No Argon2 runs for these, so caching them would only let a client
    // grow the cache with fresh random tokens.
    for i in 0..5 {
        let bearer = format!("Bearer irfl_unknown{i}");
        let req = header_request("60.0.0.4", "authorization", &bearer);
        let resp = test_app(ctx.clone()).oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "token {i}");
    }
    assert_eq!(ctx.limiter.verified_keys.len(), 0);
}

#[tokio::test]
async fn inactive_api_key_is_not_cached() {
    let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
    let (api_key_id, raw_key) = setup_api_key_in_store(&store, None, true).await;
    let deactivate = ApiKeyUpdate {
        is_active: Some(false),
        ..ApiKeyUpdate::default()
    };
    store.update_api_key(api_key_id, deactivate).await.unwrap();
    let ctx = ctx_with_store(store, 60);

    let req = header_request("60.0.0.5", "authorization", &format!("Bearer {raw_key}"));
    let resp = test_app(ctx.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(ctx.limiter.verified_keys.len(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_requests_with_a_valid_key_spend_one_verification() {
    let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
    let (_api_key_id, raw_key) = setup_api_key_in_store(&store, None, true).await;
    let ctx = ctx_with_store(store, 100);
    let bearer = format!("Bearer {raw_key}");

    // More concurrent first uses than the verification budget: they wait for
    // one verdict instead of each running Argon2.
    let mut requests = JoinSet::new();
    for _ in 0..KEY_CHECKS_PER_MINUTE * 2 {
        let req = header_request("60.0.0.6", "authorization", &bearer);
        requests.spawn(test_app(ctx.clone()).oneshot(req));
    }
    while let Some(joined) = requests.join_next().await {
        let resp = joined.unwrap().unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers()["x-ratelimit-limit"], "100");
    }
    assert_eq!(ctx.limiter.verified_keys.len(), 1);
    assert_eq!(ctx.limiter.pending_checks.len(), 0);
}

#[tokio::test]
async fn api_key_without_peer_uses_its_bucket_and_a_per_key_budget() {
    let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
    let (_api_key_id, raw_key) = setup_api_key_in_store(&store, None, true).await;
    let ctx = ctx_with_store(store, 60);
    let no_peer_bearer = |token: &str| {
        Request::builder()
            .uri("/test")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap()
    };

    let resp = test_app(ctx.clone())
        .oneshot(no_peer_bearer(&raw_key))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.headers()["x-ratelimit-limit"], "60");
    assert_eq!(remaining(&resp), 59);

    // Without a peer there is no IP bucket, but guessing still spends the
    // budget of the key. The first verification above already spent one.
    for i in 0..KEY_CHECKS_PER_MINUTE - 1 {
        let resp = test_app(ctx.clone())
            .oneshot(no_peer_bearer(&format!("irfl_abcdef12WRONG{i}")))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "guess {i}");
    }
    let resp = test_app(ctx)
        .oneshot(no_peer_bearer("irfl_abcdef12WRONGX"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
}
