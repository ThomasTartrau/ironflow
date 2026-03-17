//! Integration tests for the Prometheus metrics feature.
//!
//! These tests exercise the `/metrics` endpoint and verify that operations
//! emit Prometheus-formatted metrics. They run only when the `prometheus`
//! feature is enabled: `cargo test --features prometheus -p ironflow-runtime`.

#![cfg(feature = "prometheus")]

use std::sync::OnceLock;
use std::time::Duration;

use ironflow_runtime::prelude::*;
use metrics_exporter_prometheus::PrometheusBuilder;
use metrics_exporter_prometheus::PrometheusHandle;
use tokio::net::TcpListener;
use tokio::time::timeout;

static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();

/// Returns a shared PrometheusHandle (global recorder installed once).
fn global_handle() -> PrometheusHandle {
    HANDLE
        .get_or_init(|| {
            PrometheusBuilder::new()
                .install_recorder()
                .expect("failed to install recorder")
        })
        .clone()
}

async fn spawn_server_with_metrics(runtime: Runtime) -> String {
    let handle = global_handle();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let router = runtime.into_router().route(
        "/metrics",
        axum::routing::get(move || {
            let h = handle.clone();
            async move { h.render() }
        }),
    );

    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    format!("http://{addr}")
}

#[tokio::test]
async fn metrics_endpoint_returns_200() {
    let base = spawn_server_with_metrics(Runtime::new()).await;

    let resp = timeout(
        Duration::from_secs(10),
        reqwest::get(format!("{base}/metrics")),
    )
    .await
    .unwrap()
    .unwrap();

    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn webhook_increments_metrics() {
    let rt = Runtime::new().webhook("/hook-prom", WebhookAuth::none(), |_payload| async {});
    let base = spawn_server_with_metrics(rt).await;

    let client = reqwest::Client::new();

    let resp = timeout(
        Duration::from_secs(10),
        client
            .post(format!("{base}/hook-prom"))
            .json(&serde_json::json!({"test": true}))
            .send(),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(resp.status(), 202);

    // Give the spawned handler a moment
    tokio::time::sleep(Duration::from_millis(50)).await;

    let metrics = timeout(
        Duration::from_secs(10),
        reqwest::get(format!("{base}/metrics")),
    )
    .await
    .unwrap()
    .unwrap()
    .text()
    .await
    .unwrap();

    assert!(
        metrics.contains("ironflow_webhook_received_total"),
        "expected webhook metric in output:\n{metrics}"
    );
}

#[tokio::test]
async fn rejected_webhook_increments_rejected_metric() {
    let rt = Runtime::new().webhook(
        "/hook-reject",
        WebhookAuth::header("x-token", "correct"),
        |_payload| async {},
    );
    let base = spawn_server_with_metrics(rt).await;

    let client = reqwest::Client::new();

    let resp = timeout(
        Duration::from_secs(10),
        client
            .post(format!("{base}/hook-reject"))
            .header("x-token", "wrong")
            .json(&serde_json::json!({}))
            .send(),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(resp.status(), 401);

    let metrics = timeout(
        Duration::from_secs(10),
        reqwest::get(format!("{base}/metrics")),
    )
    .await
    .unwrap()
    .unwrap()
    .text()
    .await
    .unwrap();

    assert!(
        metrics.contains("rejected"),
        "expected 'rejected' label in metrics:\n{metrics}"
    );
}
