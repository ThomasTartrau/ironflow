use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use ironflow_runtime::prelude::*;
use tokio::time::timeout;

#[tokio::test]
async fn cron_job_fires_within_expected_window() {
    let counter = Arc::new(AtomicUsize::new(0));
    let counter_clone = counter.clone();

    let rt = Runtime::new().cron("* * * * * *", "every-second", move || {
        let counter = counter_clone.clone();
        async move {
            counter.fetch_add(1, Ordering::SeqCst);
        }
    });

    let server = tokio::spawn(async move {
        let _ = rt.run_crons().await;
    });

    tokio::time::sleep(Duration::from_secs(3)).await;

    let count = counter.load(Ordering::SeqCst);
    assert!(
        count >= 1,
        "expected at least 1 cron execution, got {count}"
    );

    server.abort();
}

#[tokio::test]
async fn cron_job_receives_correct_name_in_builder() {
    // Registering a cron with an invalid expression should fail at run_crons time.
    // This is a negative test.
    let rt = Runtime::new().cron("not-a-cron-expr", "bad-cron", || async {});

    let result = timeout(Duration::from_secs(5), rt.run_crons()).await;

    match result {
        Ok(Err(_)) => {} // Expected: invalid cron expression error.
        Ok(Ok(())) => panic!("expected an error for invalid cron expression"),
        Err(_) => panic!("run_crons timed out instead of failing fast"),
    }
}

#[tokio::test]
async fn multiple_cron_jobs_can_be_registered() {
    let counter_a = Arc::new(AtomicUsize::new(0));
    let counter_b = Arc::new(AtomicUsize::new(0));
    let ca = counter_a.clone();
    let cb = counter_b.clone();

    let rt = Runtime::new()
        .cron("* * * * * *", "job-a", move || {
            let c = ca.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
            }
        })
        .cron("* * * * * *", "job-b", move || {
            let c = cb.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
            }
        });

    let server = tokio::spawn(async move {
        let _ = rt.run_crons().await;
    });

    tokio::time::sleep(Duration::from_secs(3)).await;

    let a = counter_a.load(Ordering::SeqCst);
    let b = counter_b.load(Ordering::SeqCst);
    assert!(a >= 1, "job-a should have fired at least once, got {a}");
    assert!(b >= 1, "job-b should have fired at least once, got {b}");

    server.abort();
}

#[tokio::test]
async fn run_crons_works_without_http_server() {
    let counter = Arc::new(AtomicUsize::new(0));
    let counter_clone = counter.clone();

    // No webhooks registered, only a cron job.
    let rt = Runtime::new().cron("* * * * * *", "cron-only", move || {
        let counter = counter_clone.clone();
        async move {
            counter.fetch_add(1, Ordering::SeqCst);
        }
    });

    let handle = tokio::spawn(async move {
        let _ = rt.run_crons().await;
    });

    tokio::time::sleep(Duration::from_secs(3)).await;

    let count = counter.load(Ordering::SeqCst);
    assert!(
        count >= 1,
        "expected at least 1 cron execution without HTTP, got {count}"
    );

    handle.abort();
}

#[tokio::test]
async fn run_crons_invalid_expression_fails() {
    let rt = Runtime::new().cron("bad", "invalid", || async {});

    let result = timeout(Duration::from_secs(5), rt.run_crons()).await;

    match result {
        Ok(Err(_)) => {} // Expected: invalid cron expression error.
        Ok(Ok(())) => panic!("expected an error for invalid cron expression"),
        Err(_) => panic!("run_crons timed out instead of failing fast"),
    }
}

#[tokio::test]
async fn serve_still_starts_crons() {
    // Verify that serve() still starts the cron scheduler (regression test).
    let counter = Arc::new(AtomicUsize::new(0));
    let counter_clone = counter.clone();

    let rt = Runtime::new().cron("* * * * * *", "serve-cron", move || {
        let counter = counter_clone.clone();
        async move {
            counter.fetch_add(1, Ordering::SeqCst);
        }
    });

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let _ = rt.serve(&format!("127.0.0.1:{}", addr.port())).await;
    });

    tokio::time::sleep(Duration::from_secs(3)).await;

    let count = counter.load(Ordering::SeqCst);
    assert!(
        count >= 1,
        "serve() should still start crons, got {count} executions"
    );

    server.abort();
}
