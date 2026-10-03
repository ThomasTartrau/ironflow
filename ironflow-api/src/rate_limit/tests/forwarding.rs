//! The IP bucket follows the TCP peer, and forwarding headers only when that
//! peer is a trusted proxy (pentest AUTH-VULN-08).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use super::*;

#[tokio::test]
async fn forwarded_for_from_an_untrusted_peer_does_not_pick_the_bucket() {
    let ctx = test_ctx(1);
    for (forwarded_for, expected) in [
        ("9.9.9.1", StatusCode::OK),
        // Another forged address, same peer: same bucket.
        ("9.9.9.2", StatusCode::TOO_MANY_REQUESTS),
    ] {
        let req = header_request("203.0.113.7", "x-forwarded-for", forwarded_for);
        let resp = test_app(ctx.clone()).oneshot(req).await.unwrap();
        assert_eq!(resp.status(), expected, "x-forwarded-for {forwarded_for}");
    }
}

#[tokio::test]
async fn x_real_ip_from_an_untrusted_peer_does_not_pick_the_bucket() {
    let ctx = test_ctx(1);
    for (real_ip, expected) in [
        ("192.168.1.1", StatusCode::OK),
        ("192.168.1.2", StatusCode::TOO_MANY_REQUESTS),
    ] {
        let req = header_request("203.0.113.7", "x-real-ip", real_ip);
        let resp = test_app(ctx.clone()).oneshot(req).await.unwrap();
        assert_eq!(resp.status(), expected, "x-real-ip {real_ip}");
    }
}

#[tokio::test]
async fn trusted_proxy_forwards_each_client_to_its_own_bucket() {
    let mut ctx = test_ctx(1);
    ctx.trusted_proxies = "10.0.0.0/8".parse().unwrap();
    for (forwarded_for, expected) in [
        ("198.51.100.1", StatusCode::OK),
        ("198.51.100.2", StatusCode::OK),
        ("198.51.100.1", StatusCode::TOO_MANY_REQUESTS),
    ] {
        let req = header_request("10.0.0.2", "x-forwarded-for", forwarded_for);
        let resp = test_app(ctx.clone()).oneshot(req).await.unwrap();
        assert_eq!(resp.status(), expected, "x-forwarded-for {forwarded_for}");
    }
}

#[tokio::test]
async fn hops_forged_before_the_trusted_proxy_are_ignored() {
    let mut ctx = test_ctx(1);
    ctx.trusted_proxies = "10.0.0.0/8".parse().unwrap();
    // The proxy appends the real client; the left hop is the client's own.
    for (forwarded_for, expected) in [
        ("9.9.9.1, 198.51.100.1", StatusCode::OK),
        ("9.9.9.2, 198.51.100.1", StatusCode::TOO_MANY_REQUESTS),
    ] {
        let req = header_request("10.0.0.2", "x-forwarded-for", forwarded_for);
        let resp = test_app(ctx.clone()).oneshot(req).await.unwrap();
        assert_eq!(resp.status(), expected, "x-forwarded-for {forwarded_for}");
    }
}

#[tokio::test]
async fn x_real_ip_from_a_trusted_proxy_picks_the_bucket() {
    let mut ctx = test_ctx(1);
    ctx.trusted_proxies = "10.0.0.2".parse().unwrap();
    for (real_ip, expected) in [
        ("198.51.100.1", StatusCode::OK),
        ("198.51.100.2", StatusCode::OK),
        ("198.51.100.1", StatusCode::TOO_MANY_REQUESTS),
    ] {
        let req = header_request("10.0.0.2", "x-real-ip", real_ip);
        let resp = test_app(ctx.clone()).oneshot(req).await.unwrap();
        assert_eq!(resp.status(), expected, "x-real-ip {real_ip}");
    }
}

#[tokio::test]
async fn requests_without_peer_address_share_one_bucket() {
    let ctx = test_ctx(1);
    for (forwarded_for, expected) in [
        ("9.9.9.1", StatusCode::OK),
        ("9.9.9.2", StatusCode::TOO_MANY_REQUESTS),
    ] {
        let req = Request::builder()
            .uri("/test")
            .header("x-forwarded-for", forwarded_for)
            .body(Body::empty())
            .unwrap();
        let resp = test_app(ctx.clone()).oneshot(req).await.unwrap();
        assert_eq!(resp.status(), expected, "x-forwarded-for {forwarded_for}");
    }
}
