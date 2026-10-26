//! `metrics_addr` serves the worker metrics in the Prometheus text format.
//!
//! One test per file: the metrics recorder is process-global.

#![cfg(feature = "prometheus")]

mod helpers;

use std::net::{SocketAddr, TcpListener};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ironflow_core::metric_names::WORKER_POLLS_TOTAL;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_worker::WorkerBuilder;
use reqwest::get;
use tokio::spawn;
use tokio::time::{sleep, timeout};

use helpers::{TestApiState, spawn_test_api};

/// Wall-clock budget for the whole flow.
const TEST_TIMEOUT: Duration = Duration::from_secs(30);

/// How long the endpoint may take to come up and expose the poll counter.
const SERVE_DEADLINE: Duration = Duration::from_secs(15);

/// Reserve a free loopback port: bind port 0, read it back, release it.
fn free_loopback_addr() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind a free port");
    listener.local_addr().expect("local addr")
}

#[tokio::test]
async fn metrics_addr_serves_worker_metrics_in_prometheus_format() {
    timeout(TEST_TIMEOUT, async {
        let api_url = spawn_test_api(Arc::new(TestApiState::new(vec![]))).await;
        let addr = free_loopback_addr();

        let worker = WorkerBuilder::new(&api_url, "test-token")
            .provider(Arc::new(ClaudeCodeProvider::new()))
            .worker_id("worker-metrics")
            .concurrency(1)
            .poll_interval(Duration::from_millis(20))
            .metrics_addr(addr)
            .build()
            .expect("build worker");

        let handle = spawn(async move { worker.run().await });

        let url = format!("http://{addr}/metrics");
        let deadline = Instant::now() + SERVE_DEADLINE;
        let mut body = None;
        while Instant::now() < deadline {
            if let Ok(resp) = get(&url).await
                && resp.status().is_success()
                && let Ok(text) = resp.text().await
                && text.contains(WORKER_POLLS_TOTAL)
            {
                body = Some(text);
                break;
            }
            sleep(Duration::from_millis(50)).await;
        }
        handle.abort();

        let body = body.expect("the endpoint never exposed the worker poll counter");
        assert!(body.contains("# TYPE"), "not Prometheus text: {body}");
    })
    .await
    .expect("test timed out");
}
