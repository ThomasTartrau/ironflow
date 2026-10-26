//! Without `metrics_addr`, the worker installs no recorder and listens nowhere.
//!
//! One test per file: the metrics recorder is process-global.

#![cfg(feature = "prometheus")]

mod helpers;

use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_worker::WorkerBuilder;
use metrics_exporter_prometheus::PrometheusBuilder;
use tokio::net::TcpStream;
use tokio::spawn;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

use helpers::{TestApiState, make_run_json, spawn_test_api};

#[tokio::test]
async fn worker_without_metrics_addr_installs_no_recorder_and_listens_nowhere() {
    timeout(Duration::from_secs(30), async {
        let state = Arc::new(TestApiState::new(vec![make_run_json(
            Uuid::now_v7(),
            "unregistered",
            0,
        )]));
        let api_url = spawn_test_api(state.clone()).await;
        let addr = TcpListener::bind("127.0.0.1:0")
            .expect("bind a free port")
            .local_addr()
            .expect("local addr");

        let worker = WorkerBuilder::new(&api_url, "test-token")
            .provider(Arc::new(ClaudeCodeProvider::new()))
            .worker_id("worker-no-metrics")
            .poll_interval(Duration::from_millis(20))
            .build()
            .expect("build worker");
        let handle = spawn(async move { worker.run().await });

        let deadline = Instant::now() + Duration::from_secs(15);
        while state.handed_out.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
            sleep(Duration::from_millis(20)).await;
        }
        assert!(
            state.handed_out.load(Ordering::SeqCst) >= 1,
            "the worker never polled"
        );

        assert!(
            PrometheusBuilder::new().install_recorder().is_ok(),
            "the worker installed a recorder without metrics_addr"
        );
        assert!(
            TcpStream::connect(addr).await.is_err(),
            "something listens on {addr}"
        );
        handle.abort();
    })
    .await
    .expect("test timed out");
}
