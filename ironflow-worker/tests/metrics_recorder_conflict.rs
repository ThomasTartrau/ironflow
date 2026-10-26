//! `run` fails visibly when a metrics recorder is already installed.
//!
//! One test per file: the metrics recorder is process-global.

#![cfg(feature = "prometheus")]

mod helpers;

use std::net::TcpListener;
use std::sync::Arc;
use std::time::Duration;

use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_worker::{WorkerBuilder, WorkerError};
use metrics_exporter_prometheus::PrometheusBuilder;
use tokio::time::timeout;

use helpers::{TestApiState, spawn_test_api};

#[tokio::test]
async fn run_fails_when_a_metrics_recorder_is_already_installed() {
    PrometheusBuilder::new()
        .install_recorder()
        .expect("install the first recorder");

    let api_url = spawn_test_api(Arc::new(TestApiState::new(vec![]))).await;
    let addr = TcpListener::bind("127.0.0.1:0")
        .expect("bind a free port")
        .local_addr()
        .expect("local addr");

    let worker = WorkerBuilder::new(&api_url, "test-token")
        .provider(Arc::new(ClaudeCodeProvider::new()))
        .worker_id("worker-conflict")
        .poll_interval(Duration::from_millis(20))
        .metrics_addr(addr)
        .build()
        .expect("build worker");

    let result = timeout(Duration::from_secs(10), worker.run())
        .await
        .expect("run must fail early, not poll");

    match result {
        Err(WorkerError::Internal(msg)) => {
            assert!(msg.contains("recorder"), "unexpected message: {msg}")
        }
        other => panic!("expected WorkerError::Internal, got {other:?}"),
    }
}
