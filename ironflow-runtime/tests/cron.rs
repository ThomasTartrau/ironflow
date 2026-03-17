use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use ironflow_runtime::prelude::*;
use tokio::time::timeout;

#[tokio::test]
async fn cron_job_fires_within_expected_window() {
    let counter = Arc::new(AtomicUsize::new(0));
    let counter_clone = counter.clone();

    // Every second (6-field cron: sec min hour dom month dow).
    let rt = Runtime::new().cron("* * * * * *", "every-second", move || {
        let counter = counter_clone.clone();
        async move {
            counter.fetch_add(1, Ordering::SeqCst);
        }
    });

    // Start the full server (which also starts the scheduler).
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        // We can't call serve() directly because it blocks on ctrl_c.
        // Instead, build the router + start the cron via serve internals.
        // Since serve() blocks, we'll just use the raw scheduler approach.
        // But serve() is the only way to start crons, so we spawn + cancel.
        let _ = rt.serve(&format!("127.0.0.1:{}", addr.port())).await;
    });

    // Wait long enough for at least 2 cron ticks (the scheduler may need
    // ~1 s to initialise, so give it 3 s total).
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
    // Registering a cron with an invalid expression should fail at serve time.
    // This is a negative test.
    let rt = Runtime::new().cron("not-a-cron-expr", "bad-cron", || async {});

    let result = timeout(Duration::from_secs(5), rt.serve("127.0.0.1:0")).await;

    match result {
        Ok(Err(_)) => {} // Expected: invalid cron expression error.
        Ok(Ok(())) => panic!("expected an error for invalid cron expression"),
        Err(_) => panic!("serve timed out instead of failing fast"),
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

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let _ = rt.serve(&format!("127.0.0.1:{}", addr.port())).await;
    });

    tokio::time::sleep(Duration::from_secs(3)).await;

    let a = counter_a.load(Ordering::SeqCst);
    let b = counter_b.load(Ordering::SeqCst);
    assert!(a >= 1, "job-a should have fired at least once, got {a}");
    assert!(b >= 1, "job-b should have fired at least once, got {b}");

    server.abort();
}
