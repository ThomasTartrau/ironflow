//! Integration tests for [`HttpProbe`] with wiremock.

#![cfg(feature = "trigger-polling-http")]

use std::time::Duration;

use serde_json::json;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use ironflow_runtime::trigger::polling::http::{HttpProbe, HttpProbeConfig};
use ironflow_runtime::trigger::polling::{PollingTrigger, PollingTriggerConfig};
use ironflow_runtime::trigger::{Trigger, TriggerSink};

#[tokio::test]
async fn http_probe_triggers_on_body_change() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/data"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"version": 1})))
        .up_to_n_times(1)
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/data"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"version": 2})))
        .mount(&server)
        .await;

    let probe = HttpProbe::new(HttpProbeConfig {
        url: format!("{}/data", server.uri()),
        method: "GET".to_string(),
        headers: vec![],
        expected_status: 200,
    });

    let trigger = PollingTrigger::new(PollingTriggerConfig {
        interval: Duration::from_millis(50),
        probe: Box::new(probe),
        workflow_name: "test-http".to_string(),
        dedup: true,
    });

    let (sink, mut rx) = TriggerSink::channel(16);
    let token = CancellationToken::new();
    let token_clone = token.clone();

    let handle = tokio::spawn(async move { trigger.start(sink, &token_clone).await });

    // First poll: version 1
    let e1 = timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("timed out waiting for first event")
        .expect("channel closed");
    assert_eq!(e1.workflow_name, "test-http");
    assert_eq!(e1.payload["version"], 1);

    // Second poll: version 2 (body changed, should trigger again)
    let e2 = timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("timed out waiting for second event")
        .expect("channel closed");
    assert_eq!(e2.payload["version"], 2);

    token.cancel();
    let _ = handle.await;
}

#[tokio::test]
async fn http_probe_dedup_skips_identical_body() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/stable"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"stable": true})))
        .expect(2..)
        .mount(&server)
        .await;

    let probe = HttpProbe::new(HttpProbeConfig {
        url: format!("{}/stable", server.uri()),
        method: "GET".to_string(),
        headers: vec![],
        expected_status: 200,
    });

    let trigger = PollingTrigger::new(PollingTriggerConfig {
        interval: Duration::from_millis(50),
        probe: Box::new(probe),
        workflow_name: "test-stable".to_string(),
        dedup: true,
    });

    let (sink, mut rx) = TriggerSink::channel(16);
    let token = CancellationToken::new();
    let token_clone = token.clone();

    let handle = tokio::spawn(async move { trigger.start(sink, &token_clone).await });

    // First poll triggers
    let _e1 = timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("timed out")
        .expect("channel closed");

    // Wait for a couple more poll cycles
    tokio::time::sleep(Duration::from_millis(200)).await;

    // No second event should arrive (body unchanged)
    assert!(rx.try_recv().is_err());

    token.cancel();
    let _ = handle.await;
}

#[tokio::test]
async fn http_probe_error_on_unexpected_status() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/fail"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let probe = HttpProbe::new(HttpProbeConfig {
        url: format!("{}/fail", server.uri()),
        method: "GET".to_string(),
        headers: vec![],
        expected_status: 200,
    });

    let trigger = PollingTrigger::new(PollingTriggerConfig {
        interval: Duration::from_millis(50),
        probe: Box::new(probe),
        workflow_name: "test-fail".to_string(),
        dedup: false,
    });

    let (sink, mut rx) = TriggerSink::channel(16);
    let token = CancellationToken::new();
    let token_clone = token.clone();

    let handle = tokio::spawn(async move { trigger.start(sink, &token_clone).await });

    // Wait for a few poll cycles
    tokio::time::sleep(Duration::from_millis(200)).await;

    // No event should have been emitted (status mismatch)
    assert!(rx.try_recv().is_err());

    token.cancel();
    let _ = handle.await;
}

#[tokio::test]
async fn http_probe_sends_custom_headers() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/auth"))
        .and(wiremock::matchers::header("X-Api-Key", "secret"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .mount(&server)
        .await;

    let probe = HttpProbe::new(HttpProbeConfig {
        url: format!("{}/auth", server.uri()),
        method: "GET".to_string(),
        headers: vec![("X-Api-Key".to_string(), "secret".to_string())],
        expected_status: 200,
    });

    let trigger = PollingTrigger::new(PollingTriggerConfig {
        interval: Duration::from_millis(50),
        probe: Box::new(probe),
        workflow_name: "test-auth".to_string(),
        dedup: false,
    });

    let (sink, mut rx) = TriggerSink::channel(16);
    let token = CancellationToken::new();
    let token_clone = token.clone();

    let handle = tokio::spawn(async move { trigger.start(sink, &token_clone).await });

    let event = timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("timed out")
        .expect("channel closed");

    assert_eq!(event.payload["ok"], true);

    token.cancel();
    let _ = handle.await;
}
