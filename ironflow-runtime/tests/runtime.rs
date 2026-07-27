use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hmac::{Hmac, Mac};
use ironflow_runtime::prelude::*;
use sha2::Sha256;
use tokio::net::TcpListener;
use tokio::time::timeout;

type HmacSha256 = Hmac<Sha256>;

async fn send_with_timeout(
    future: impl std::future::Future<Output = Result<reqwest::Response, reqwest::Error>>,
) -> reqwest::Response {
    timeout(Duration::from_secs(10), future)
        .await
        .expect("request timed out")
        .expect("request failed")
}

async fn spawn_server(runtime: Runtime) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = runtime.into_router();

    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    format!("http://{addr}")
}

#[tokio::test]
async fn health_endpoint_returns_200_with_ok() {
    let base = spawn_server(Runtime::new()).await;
    let resp = send_with_timeout(reqwest::get(format!("{base}/health"))).await;
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.unwrap(), "ok");
}

#[tokio::test]
async fn webhook_none_auth_valid_json_returns_202() {
    let rt = Runtime::new().webhook("/hook", WebhookAuth::none(), |_payload| async {});
    let base = spawn_server(rt).await;

    let client = reqwest::Client::new();
    let resp = send_with_timeout(
        client
            .post(format!("{base}/hook"))
            .json(&serde_json::json!({"key": "value"}))
            .send(),
    )
    .await;
    assert_eq!(resp.status(), 202);
}

#[tokio::test]
async fn webhook_invalid_json_returns_400() {
    let rt = Runtime::new().webhook("/hook", WebhookAuth::none(), |_payload| async {});
    let base = spawn_server(rt).await;

    let client = reqwest::Client::new();
    let resp = send_with_timeout(
        client
            .post(format!("{base}/hook"))
            .header("content-type", "application/json")
            .body("not json")
            .send(),
    )
    .await;
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn webhook_header_auth_valid_returns_202() {
    let rt = Runtime::new().webhook(
        "/hook",
        WebhookAuth::header("x-api-key", "secret"),
        |_payload| async {},
    );
    let base = spawn_server(rt).await;

    let client = reqwest::Client::new();
    let resp = send_with_timeout(
        client
            .post(format!("{base}/hook"))
            .header("x-api-key", "secret")
            .json(&serde_json::json!({}))
            .send(),
    )
    .await;
    assert_eq!(resp.status(), 202);
}

#[tokio::test]
async fn webhook_header_auth_invalid_returns_401() {
    let rt = Runtime::new().webhook(
        "/hook",
        WebhookAuth::header("x-api-key", "secret"),
        |_payload| async {},
    );
    let base = spawn_server(rt).await;

    let client = reqwest::Client::new();
    let resp = send_with_timeout(
        client
            .post(format!("{base}/hook"))
            .header("x-api-key", "wrong")
            .json(&serde_json::json!({}))
            .send(),
    )
    .await;
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn webhook_header_auth_missing_returns_401() {
    let rt = Runtime::new().webhook(
        "/hook",
        WebhookAuth::header("x-api-key", "secret"),
        |_payload| async {},
    );
    let base = spawn_server(rt).await;

    let client = reqwest::Client::new();
    let resp = send_with_timeout(
        client
            .post(format!("{base}/hook"))
            .json(&serde_json::json!({}))
            .send(),
    )
    .await;
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn webhook_handler_receives_posted_json() {
    let received = Arc::new(AtomicBool::new(false));
    let received_clone = received.clone();

    let rt = Runtime::new().webhook("/hook", WebhookAuth::none(), move |payload| {
        let received = received_clone.clone();
        async move {
            if payload.get("msg").and_then(|v| v.as_str()) == Some("hello") {
                received.store(true, Ordering::SeqCst);
            }
        }
    });
    let base = spawn_server(rt).await;

    let client = reqwest::Client::new();
    let resp = send_with_timeout(
        client
            .post(format!("{base}/hook"))
            .json(&serde_json::json!({"msg": "hello"}))
            .send(),
    )
    .await;
    assert_eq!(resp.status(), 202);

    // The handler runs in a spawned task; give it a moment to complete.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(received.load(Ordering::SeqCst));
}

#[tokio::test]
async fn multiple_webhooks_on_different_paths() {
    let rt = Runtime::new()
        .webhook("/hook-a", WebhookAuth::none(), |_p| async {})
        .webhook("/hook-b", WebhookAuth::none(), |_p| async {});
    let base = spawn_server(rt).await;

    let client = reqwest::Client::new();

    let resp_a = send_with_timeout(
        client
            .post(format!("{base}/hook-a"))
            .json(&serde_json::json!({}))
            .send(),
    )
    .await;
    assert_eq!(resp_a.status(), 202);

    let resp_b = send_with_timeout(
        client
            .post(format!("{base}/hook-b"))
            .json(&serde_json::json!({}))
            .send(),
    )
    .await;
    assert_eq!(resp_b.status(), 202);
}

#[tokio::test]
async fn unknown_path_returns_404() {
    let rt = Runtime::new().webhook("/hook", WebhookAuth::none(), |_p| async {});
    let base = spawn_server(rt).await;

    let client = reqwest::Client::new();
    let resp = send_with_timeout(
        client
            .post(format!("{base}/nonexistent"))
            .json(&serde_json::json!({}))
            .send(),
    )
    .await;
    assert_eq!(resp.status(), 404);
}

fn compute_github_signature(secret: &str, body: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(body);
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}

#[tokio::test]
async fn webhook_github_hmac_valid_returns_202() {
    let secret = "gh-webhook-secret";
    let rt = Runtime::new().webhook("/hook", WebhookAuth::github(secret), |_payload| async {});
    let base = spawn_server(rt).await;

    let body = serde_json::json!({"action": "push"});
    let body_bytes = serde_json::to_vec(&body).unwrap();
    let signature = compute_github_signature(secret, &body_bytes);

    let client = reqwest::Client::new();
    let resp = send_with_timeout(
        client
            .post(format!("{base}/hook"))
            .header("content-type", "application/json")
            .header("x-hub-signature-256", &signature)
            .body(body_bytes)
            .send(),
    )
    .await;
    assert_eq!(resp.status(), 202);
}

#[tokio::test]
async fn webhook_github_hmac_invalid_returns_401() {
    let rt = Runtime::new().webhook(
        "/hook",
        WebhookAuth::github("correct-secret"),
        |_payload| async {},
    );
    let base = spawn_server(rt).await;

    let body = serde_json::json!({"action": "push"});
    let body_bytes = serde_json::to_vec(&body).unwrap();
    let wrong_signature = compute_github_signature("wrong-secret", &body_bytes);

    let client = reqwest::Client::new();
    let resp = send_with_timeout(
        client
            .post(format!("{base}/hook"))
            .header("content-type", "application/json")
            .header("x-hub-signature-256", &wrong_signature)
            .body(body_bytes)
            .send(),
    )
    .await;
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn webhook_github_hmac_missing_header_returns_401() {
    let rt = Runtime::new().webhook(
        "/hook",
        WebhookAuth::github("some-secret"),
        |_payload| async {},
    );
    let base = spawn_server(rt).await;

    let client = reqwest::Client::new();
    let resp = send_with_timeout(
        client
            .post(format!("{base}/hook"))
            .json(&serde_json::json!({"action": "push"}))
            .send(),
    )
    .await;
    assert_eq!(resp.status(), 401);
}

// ── WebhookAuth::gitlab ────────────────────────────────────────

#[tokio::test]
async fn webhook_gitlab_auth_valid_returns_202() {
    let secret = "gl-secret-token";
    let rt = Runtime::new().webhook("/hook", WebhookAuth::gitlab(secret), |_payload| async {});
    let base = spawn_server(rt).await;

    let client = reqwest::Client::new();
    let resp = send_with_timeout(
        client
            .post(format!("{base}/hook"))
            .header("x-gitlab-token", secret)
            .json(&serde_json::json!({"object_kind": "merge_request"}))
            .send(),
    )
    .await;
    assert_eq!(resp.status(), 202);
}

#[tokio::test]
async fn webhook_gitlab_auth_invalid_returns_401() {
    let rt = Runtime::new().webhook(
        "/hook",
        WebhookAuth::gitlab("correct-token"),
        |_payload| async {},
    );
    let base = spawn_server(rt).await;

    let client = reqwest::Client::new();
    let resp = send_with_timeout(
        client
            .post(format!("{base}/hook"))
            .header("x-gitlab-token", "wrong-token")
            .json(&serde_json::json!({}))
            .send(),
    )
    .await;
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn webhook_gitlab_auth_missing_header_returns_401() {
    let rt = Runtime::new().webhook(
        "/hook",
        WebhookAuth::gitlab("some-token"),
        |_payload| async {},
    );
    let base = spawn_server(rt).await;

    let client = reqwest::Client::new();
    let resp = send_with_timeout(
        client
            .post(format!("{base}/hook"))
            .json(&serde_json::json!({}))
            .send(),
    )
    .await;
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn webhook_gitlab_handler_receives_payload() {
    let received = Arc::new(AtomicBool::new(false));
    let received_clone = received.clone();
    let secret = "gl-test-token";

    let rt = Runtime::new().webhook("/hook", WebhookAuth::gitlab(secret), move |payload| {
        let received = received_clone.clone();
        async move {
            if payload.get("object_kind").and_then(|v| v.as_str()) == Some("push") {
                received.store(true, Ordering::SeqCst);
            }
        }
    });
    let base = spawn_server(rt).await;

    let client = reqwest::Client::new();
    let resp = send_with_timeout(
        client
            .post(format!("{base}/hook"))
            .header("x-gitlab-token", secret)
            .json(&serde_json::json!({"object_kind": "push"}))
            .send(),
    )
    .await;
    assert_eq!(resp.status(), 202);

    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(received.load(Ordering::SeqCst));
}

// ── Body size limit ────────────────────────────────────────────

#[tokio::test]
async fn webhook_oversized_body_is_rejected() {
    let rt = Runtime::new().max_body_size(1024).webhook(
        "/hook",
        WebhookAuth::none(),
        |_payload| async {},
    );
    let base = spawn_server(rt).await;

    // Send a body larger than 1024 bytes.
    let large_body = "x".repeat(2048);
    let client = reqwest::Client::new();
    let resp = send_with_timeout(
        client
            .post(format!("{base}/hook"))
            .header("content-type", "application/json")
            .body(format!("{{\"data\": \"{large_body}\"}}"))
            .send(),
    )
    .await;
    assert_eq!(resp.status(), 413);
}

// ── Concurrency limit ──────────────────────────────────────────

#[tokio::test]
async fn concurrent_handlers_are_bounded() {
    use std::sync::atomic::AtomicUsize;

    let active = Arc::new(AtomicUsize::new(0));
    let max_seen = Arc::new(AtomicUsize::new(0));
    let active_c = active.clone();
    let max_seen_c = max_seen.clone();

    let rt = Runtime::new().max_concurrent_handlers(2).webhook(
        "/hook",
        WebhookAuth::none(),
        move |_payload| {
            let active = active_c.clone();
            let max_seen = max_seen_c.clone();
            async move {
                let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                // Update max seen concurrency.
                max_seen.fetch_max(current, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(200)).await;
                active.fetch_sub(1, Ordering::SeqCst);
            }
        },
    );
    let base = spawn_server(rt).await;

    let client = reqwest::Client::new();
    // Fire 4 requests concurrently.
    let mut handles = Vec::new();
    for _ in 0..4 {
        let client = client.clone();
        let base = base.clone();
        handles.push(tokio::spawn(async move {
            client
                .post(format!("{base}/hook"))
                .json(&serde_json::json!({}))
                .send()
                .await
                .unwrap()
        }));
    }
    for h in handles {
        let resp = timeout(Duration::from_secs(10), h).await.unwrap().unwrap();
        assert_eq!(resp.status(), 202);
    }

    // Wait for all handlers to finish.
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(max_seen.load(Ordering::SeqCst) <= 2);
}

// ── Misc ───────────────────────────────────────────────────────

#[test]
#[should_panic(expected = "webhook path must start with '/'")]
fn webhook_path_without_slash_panics() {
    let _ = Runtime::new().webhook("no-slash", WebhookAuth::none(), |_| async {});
}

#[test]
#[should_panic(expected = "max_concurrent_handlers must be greater than 0")]
fn max_concurrent_handlers_zero_panics() {
    let _ = Runtime::new().max_concurrent_handlers(0);
}

#[tokio::test]
async fn get_on_webhook_returns_405() {
    let rt = Runtime::new().webhook("/hook", WebhookAuth::none(), |_| async {});
    let base = spawn_server(rt).await;
    let client = reqwest::Client::new();
    let resp = send_with_timeout(client.get(format!("{base}/hook")).send()).await;
    assert_eq!(resp.status(), 405);
}

#[tokio::test]
async fn webhook_empty_body_returns_400() {
    let rt = Runtime::new().webhook("/hook", WebhookAuth::none(), |_| async {});
    let base = spawn_server(rt).await;
    let client = reqwest::Client::new();
    let resp = send_with_timeout(client.post(format!("{base}/hook")).body("").send()).await;
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn runtime_default_works_same_as_new() {
    let base = spawn_server(Runtime::default()).await;
    let resp = send_with_timeout(reqwest::get(format!("{base}/health"))).await;
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.unwrap(), "ok");
}

// ── webhook_with_context / delivery id ─────────────────────────

/// Collects the delivery ids seen by a webhook handler.
#[derive(Clone, Default)]
struct DeliveryLog(Arc<Mutex<Vec<Option<String>>>>);

impl DeliveryLog {
    fn record(&self, delivery_id: Option<String>) {
        self.0.lock().unwrap().push(delivery_id);
    }

    fn seen(&self) -> Vec<Option<String>> {
        self.0.lock().unwrap().clone()
    }
}

#[tokio::test]
async fn webhook_with_context_exposes_the_github_delivery_id() {
    let log = DeliveryLog::default();
    let handler_log = log.clone();
    let rt = Runtime::new().webhook_with_context("/hook", WebhookAuth::none(), move |ctx| {
        let log = handler_log.clone();
        async move { log.record(ctx.delivery_id) }
    });
    let base = spawn_server(rt).await;

    let client = reqwest::Client::new();
    let resp = send_with_timeout(
        client
            .post(format!("{base}/hook"))
            .header("x-github-delivery", "abc-123")
            .json(&serde_json::json!({"action": "opened"}))
            .send(),
    )
    .await;
    assert_eq!(resp.status(), 202);

    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(log.seen(), vec![Some("github:abc-123".to_string())]);
}

#[tokio::test]
async fn webhook_with_context_exposes_the_gitlab_event_uuid() {
    let log = DeliveryLog::default();
    let handler_log = log.clone();
    let rt = Runtime::new().webhook_with_context("/hook", WebhookAuth::none(), move |ctx| {
        let log = handler_log.clone();
        async move { log.record(ctx.delivery_id) }
    });
    let base = spawn_server(rt).await;

    let client = reqwest::Client::new();
    send_with_timeout(
        client
            .post(format!("{base}/hook"))
            .header("x-gitlab-event-uuid", "def-456")
            .json(&serde_json::json!({"object_kind": "push"}))
            .send(),
    )
    .await;

    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(log.seen(), vec![Some("gitlab:def-456".to_string())]);
}

#[tokio::test]
async fn webhook_with_context_without_a_delivery_header_yields_none() {
    let log = DeliveryLog::default();
    let handler_log = log.clone();
    let rt = Runtime::new().webhook_with_context("/hook", WebhookAuth::none(), move |ctx| {
        let log = handler_log.clone();
        async move { log.record(ctx.delivery_id) }
    });
    let base = spawn_server(rt).await;

    let client = reqwest::Client::new();
    send_with_timeout(
        client
            .post(format!("{base}/hook"))
            .json(&serde_json::json!({}))
            .send(),
    )
    .await;

    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(log.seen(), vec![None]);
}

#[tokio::test]
async fn replayed_github_delivery_yields_the_same_key_every_time() {
    let log = DeliveryLog::default();
    let handler_log = log.clone();
    let rt = Runtime::new().webhook_with_context("/hook", WebhookAuth::none(), move |ctx| {
        let log = handler_log.clone();
        async move { log.record(ctx.delivery_id) }
    });
    let base = spawn_server(rt).await;

    let client = reqwest::Client::new();
    for _ in 0..3 {
        send_with_timeout(
            client
                .post(format!("{base}/hook"))
                .header("x-github-delivery", "replayed-delivery")
                .json(&serde_json::json!({"action": "opened"}))
                .send(),
        )
        .await;
    }

    tokio::time::sleep(Duration::from_millis(150)).await;
    let seen = log.seen();
    assert_eq!(seen.len(), 3);
    assert!(
        seen.iter()
            .all(|id| id.as_deref() == Some("github:replayed-delivery")),
        "a replayed delivery must always derive the same idempotency key: {seen:?}"
    );
}

#[tokio::test]
async fn webhook_still_receives_the_payload_only() {
    // Non-regression: the original `webhook()` signature is unchanged.
    let received = Arc::new(Mutex::new(None::<serde_json::Value>));
    let handler_received = received.clone();
    let rt = Runtime::new().webhook("/hook", WebhookAuth::none(), move |payload| {
        let received = handler_received.clone();
        async move {
            *received.lock().unwrap() = Some(payload);
        }
    });
    let base = spawn_server(rt).await;

    let client = reqwest::Client::new();
    send_with_timeout(
        client
            .post(format!("{base}/hook"))
            .header("x-github-delivery", "abc-123")
            .json(&serde_json::json!({"action": "opened"}))
            .send(),
    )
    .await;

    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        received.lock().unwrap().clone(),
        Some(serde_json::json!({"action": "opened"}))
    );
}
