use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use ironflow_core::prelude::*;

// ── HTTP retry tests ────────────────────────────────────────────────

#[tokio::test]
async fn http_retry_succeeds_after_503() {
    use tokio::net::TcpListener;

    let attempt = Arc::new(AtomicU32::new(0));
    let attempt_clone = attempt.clone();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let server = tokio::spawn(async move {
        // Serve 3 requests: first two return 503, third returns 200
        for _ in 0..3 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let current = attempt_clone.fetch_add(1, Ordering::SeqCst);

            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut buf = [0u8; 1024];
            let _ = socket.read(&mut buf).await.unwrap();

            let response = if current < 2 {
                "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 11\r\n\r\nunavailable"
            } else {
                "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok"
            };
            socket.write_all(response.as_bytes()).await.unwrap();
            socket.shutdown().await.unwrap();
        }
    });

    let url = format!("http://localhost:{port}/test");
    let output = Http::get(&url)
        .timeout(Duration::from_secs(5))
        .retry_policy(RetryPolicy::new(3).backoff(Duration::from_millis(10)))
        .await
        .unwrap();

    assert_eq!(output.status(), 200);
    assert_eq!(output.body(), "ok");
    assert_eq!(attempt.load(Ordering::SeqCst), 3);

    server.await.unwrap();
}

#[tokio::test]
async fn http_retry_returns_last_503_when_exhausted() {
    use tokio::net::TcpListener;

    let attempt = Arc::new(AtomicU32::new(0));
    let attempt_clone = attempt.clone();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    // All 3 requests return 503
    let server = tokio::spawn(async move {
        for _ in 0..3 {
            let (mut socket, _) = listener.accept().await.unwrap();
            attempt_clone.fetch_add(1, Ordering::SeqCst);

            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut buf = [0u8; 1024];
            let _ = socket.read(&mut buf).await.unwrap();

            let response =
                "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 11\r\n\r\nunavailable";
            socket.write_all(response.as_bytes()).await.unwrap();
            socket.shutdown().await.unwrap();
        }
    });

    let url = format!("http://localhost:{port}/test");
    let output = Http::get(&url)
        .timeout(Duration::from_secs(5))
        .retry_policy(RetryPolicy::new(2).backoff(Duration::from_millis(10)))
        .await
        .unwrap(); // 503 is Ok, not Err

    assert_eq!(output.status(), 503);
    assert_eq!(attempt.load(Ordering::SeqCst), 3); // 1 initial + 2 retries

    server.await.unwrap();
}

#[tokio::test]
async fn http_retry_on_429_then_success() {
    use tokio::net::TcpListener;

    let attempt = Arc::new(AtomicU32::new(0));
    let attempt_clone = attempt.clone();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let server = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let current = attempt_clone.fetch_add(1, Ordering::SeqCst);

            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut buf = [0u8; 1024];
            let _ = socket.read(&mut buf).await.unwrap();

            let response = if current < 1 {
                "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 1\r\nContent-Length: 10\r\n\r\nrate limit"
            } else {
                "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok"
            };
            socket.write_all(response.as_bytes()).await.unwrap();
            socket.shutdown().await.unwrap();
        }
    });

    let url = format!("http://localhost:{port}/test");
    let output = Http::get(&url)
        .timeout(Duration::from_secs(5))
        .retry_policy(RetryPolicy::new(2).backoff(Duration::from_millis(10)))
        .await
        .unwrap();

    assert_eq!(output.status(), 200);
    assert_eq!(attempt.load(Ordering::SeqCst), 2);

    server.await.unwrap();
}

#[tokio::test]
async fn http_no_retry_on_404() {
    use tokio::net::TcpListener;

    let attempt = Arc::new(AtomicU32::new(0));
    let attempt_clone = attempt.clone();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        attempt_clone.fetch_add(1, Ordering::SeqCst);

        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut buf = [0u8; 1024];
        let _ = socket.read(&mut buf).await.unwrap();

        let response = "HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\n\r\nnot found";
        socket.write_all(response.as_bytes()).await.unwrap();
        socket.shutdown().await.unwrap();
    });

    let url = format!("http://localhost:{port}/test");
    let output = Http::get(&url)
        .timeout(Duration::from_secs(5))
        .retry_policy(RetryPolicy::new(3).backoff(Duration::from_millis(10)))
        .await
        .unwrap();

    assert_eq!(output.status(), 404);
    assert_eq!(attempt.load(Ordering::SeqCst), 1); // no retry

    server.await.unwrap();
}

#[tokio::test]
async fn http_no_retry_without_policy() {
    use tokio::net::TcpListener;

    let attempt = Arc::new(AtomicU32::new(0));
    let attempt_clone = attempt.clone();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        attempt_clone.fetch_add(1, Ordering::SeqCst);

        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut buf = [0u8; 1024];
        let _ = socket.read(&mut buf).await.unwrap();

        let response =
            "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 11\r\n\r\nunavailable";
        socket.write_all(response.as_bytes()).await.unwrap();
        socket.shutdown().await.unwrap();
    });

    let url = format!("http://localhost:{port}/test");
    let output = Http::get(&url)
        .timeout(Duration::from_secs(5))
        .await
        .unwrap();

    assert_eq!(output.status(), 503);
    assert_eq!(attempt.load(Ordering::SeqCst), 1); // no retry

    server.await.unwrap();
}

// ── HTTP retry with dry-run ─────────────────────────────────────────

#[tokio::test]
async fn http_retry_skipped_in_dry_run() {
    let output = Http::get("https://example.com/api")
        .retry(3)
        .dry_run(true)
        .await
        .unwrap();

    assert_eq!(output.status(), 200);
    assert_eq!(output.body(), "");
}
