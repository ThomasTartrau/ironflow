//! The account bucket of the credential routes.

use axum::http::StatusCode;
use http_body_util::BodyExt;
use serde_json::{Value as JsonValue, from_slice};
use tower::ServiceExt;

use super::*;

fn by_email(burst: u32) -> RateLimitContext {
    let mut ctx = test_ctx(burst);
    ctx.account_limit = AccountLimit::ByEmail;
    ctx
}

#[tokio::test]
async fn account_limit_counts_one_email_across_addresses() {
    let ctx = by_email(2);
    let attempts = [
        (
            r#"{"email":"alice@ironflow.dev","password":"a"}"#,
            StatusCode::OK,
        ),
        (
            r#"{"email":" ALICE@ironflow.dev","password":"b"}"#,
            StatusCode::OK,
        ),
        (
            r#"{"email":"alice@IRONFLOW.dev ","password":"c"}"#,
            StatusCode::TOO_MANY_REQUESTS,
        ),
    ];
    for (i, (body, expected)) in attempts.into_iter().enumerate() {
        let peer = format!("198.51.100.{}", i + 1);
        let resp = test_app(ctx.clone())
            .oneshot(email_request(&peer, body))
            .await
            .unwrap();
        assert_eq!(resp.status(), expected, "attempt {}", i + 1);
    }
}

#[tokio::test]
async fn account_limit_off_does_not_count_emails() {
    let ctx = test_ctx(1);
    for i in 1..=3 {
        let peer = format!("198.51.100.{i}");
        let resp = test_app(ctx.clone())
            .oneshot(email_request(&peer, r#"{"email":"alice@ironflow.dev"}"#))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
}

#[tokio::test]
async fn account_limit_hands_the_body_to_the_handler_unchanged() {
    let ctx = by_email(5);
    for body in [
        r#"{"email":"bob@ironflow.dev","password":"pw"}"#,
        "not json",
    ] {
        let resp = test_app(ctx.clone())
            .oneshot(email_request("198.51.100.1", body))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let echoed = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(echoed, body.as_bytes());
    }
}

#[tokio::test]
async fn account_limit_reports_the_tightest_bucket() {
    let ctx = by_email(3);
    let body = r#"{"email":"carol@ironflow.dev"}"#;
    let first = test_app(ctx.clone())
        .oneshot(email_request("198.51.100.1", body))
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    // New address, so its bucket has 2 left; the account has 1 left.
    let resp = test_app(ctx)
        .oneshot(email_request("198.51.100.2", body))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.headers()["x-ratelimit-remaining"], "1");
}

#[tokio::test]
async fn account_limit_refuses_an_oversized_body() {
    let ctx = by_email(5);
    let body = format!(r#"{{"email":"{}"}}"#, "a".repeat(ACCOUNT_BODY_LIMIT));
    let resp = test_app(ctx)
        .oneshot(email_request("198.51.100.1", &body))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json_val: JsonValue = from_slice(&bytes).unwrap();
    assert_eq!(json_val["error"]["code"], "PAYLOAD_TOO_LARGE");
}

#[tokio::test]
async fn expired_counters_are_swept_past_the_threshold() {
    let ctx = test_ctx(1);
    let stale = now_epoch() - WINDOW_SECS - 1;
    for i in 0..SWEEP_THRESHOLD {
        ctx.limiter.counters.insert(
            RateLimitKey::Account(format!("user{i}@x.dev")),
            WindowEntry {
                count: AtomicU32::new(1),
                window_start: AtomicU64::new(stale),
            },
        );
    }
    let resp = test_app(ctx.clone())
        .oneshot(ip_request("198.51.100.1"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(ctx.limiter.counters.len(), 1);
}
