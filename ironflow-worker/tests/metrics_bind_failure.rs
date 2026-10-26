//! `run` fails visibly when the metrics address cannot be bound.
//!
//! One test per file: the metrics recorder is process-global.

#![cfg(feature = "prometheus")]

mod helpers;

use std::net::TcpListener;
use std::sync::Arc;
use std::time::Duration;

use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_worker::{WorkerBuilder, WorkerError};
use tokio::time::timeout;

use helpers::{TestApiState, spawn_test_api};

#[tokio::test]
async fn run_fails_when_the_metrics_address_is_already_in_use() {
    let api_url = spawn_test_api(Arc::new(TestApiState::new(vec![]))).await;
    // Keep the listener alive so the port stays taken for the whole test.
    let holder = TcpListener::bind("127.0.0.1:0").expect("bind a free port");
    let addr = holder.local_addr().expect("local addr");

    let worker = WorkerBuilder::new(&api_url, "test-token")
        .provider(Arc::new(ClaudeCodeProvider::new()))
        .worker_id("worker-bind-failure")
        .poll_interval(Duration::from_millis(20))
        .metrics_addr(addr)
        .build()
        .expect("build worker");

    let result = timeout(Duration::from_secs(10), worker.run())
        .await
        .expect("run must fail early, not poll");

    match result {
        Err(WorkerError::Internal(msg)) => {
            assert!(msg.contains(&addr.to_string()), "unexpected message: {msg}")
        }
        other => panic!("expected WorkerError::Internal, got {other:?}"),
    }
    drop(holder);
}
