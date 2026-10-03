use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use super::*;

#[test]
fn get_builder_sets_method_and_url() {
    let http = Http::get("https://example.com");
    assert_eq!(http.method, Method::GET);
    assert_eq!(http.url, "https://example.com");
}

#[test]
fn post_builder_sets_method() {
    let http = Http::post("https://example.com");
    assert_eq!(http.method, Method::POST);
}

#[test]
fn put_builder_sets_method() {
    assert_eq!(Http::put("https://x.com").method, Method::PUT);
}

#[test]
fn patch_builder_sets_method() {
    assert_eq!(Http::patch("https://x.com").method, Method::PATCH);
}

#[test]
fn delete_builder_sets_method() {
    assert_eq!(Http::delete("https://x.com").method, Method::DELETE);
}

#[test]
fn header_builder_stores_headers() {
    let http = Http::get("https://x.com")
        .header("Authorization", "Bearer token")
        .header("Accept", "application/json");
    assert_eq!(http.headers.get("Authorization").unwrap(), "Bearer token");
    assert_eq!(http.headers.get("Accept").unwrap(), "application/json");
}

#[test]
fn timeout_builder_stores_duration() {
    let http = Http::get("https://x.com").timeout(Duration::from_secs(60));
    assert_eq!(http.timeout, Some(Duration::from_secs(60)));
}

#[test]
fn default_timeout_is_30_seconds() {
    let http = Http::get("https://x.com");
    assert_eq!(http.timeout, Some(DEFAULT_HTTP_TIMEOUT));
}

#[test]
fn http_output_is_success_for_2xx() {
    for status in [200, 201, 202, 204, 299] {
        let output = HttpOutput {
            status,
            headers: HashMap::new(),
            body: String::new(),
            duration_ms: 0,
        };
        assert!(output.is_success(), "expected {status} to be success");
    }
}

#[test]
fn http_output_is_not_success_for_non_2xx() {
    for status in [100, 301, 400, 401, 403, 404, 500, 503] {
        let output = HttpOutput {
            status,
            headers: HashMap::new(),
            body: String::new(),
            duration_ms: 0,
        };
        assert!(!output.is_success(), "expected {status} to not be success");
    }
}

#[test]
fn http_output_json_parses_valid_json() {
    let output = HttpOutput {
        status: 200,
        headers: HashMap::new(),
        body: r#"{"name":"test","count":42}"#.to_string(),
        duration_ms: 0,
    };
    let parsed: serde_json::Value = output.json().unwrap();
    assert_eq!(parsed["name"], "test");
    assert_eq!(parsed["count"], 42);
}

#[test]
fn http_output_json_fails_on_invalid_json() {
    let output = HttpOutput {
        status: 200,
        headers: HashMap::new(),
        body: "not json".to_string(),
        duration_ms: 0,
    };
    let err = output.json::<serde_json::Value>().unwrap_err();
    assert!(matches!(err, OperationError::Deserialize { .. }));
}

#[test]
#[should_panic(expected = "url must not be empty")]
fn empty_url_panics() {
    let _ = Http::get("");
}

#[test]
#[should_panic(expected = "url must not be empty")]
fn whitespace_url_panics() {
    let _ = Http::post("   ");
}

#[test]
#[should_panic(expected = "url must use http:// or https://")]
fn non_http_scheme_panics() {
    let _ = Http::get("file:///etc/passwd");
}

#[test]
#[should_panic(expected = "url must use http:// or https://")]
fn ftp_scheme_panics() {
    let _ = Http::get("ftp://example.com");
}

#[tokio::test]
async fn ssrf_localhost_blocked() {
    let err = Http::get("http://127.0.0.1/secret")
        .run()
        .await
        .unwrap_err();
    assert!(err.to_string().contains("blocked IP address"));
}

#[tokio::test]
async fn ssrf_metadata_blocked() {
    let err = Http::get("http://169.254.169.254/latest/meta-data/")
        .run()
        .await
        .unwrap_err();
    assert!(err.to_string().contains("blocked IP address"));
}

#[tokio::test]
async fn ssrf_private_10_blocked() {
    let err = Http::get("http://10.0.0.1/internal")
        .run()
        .await
        .unwrap_err();
    assert!(err.to_string().contains("blocked IP address"));
}

#[tokio::test]
async fn ssrf_ipv6_loopback_blocked() {
    let err = Http::get("http://[::1]/secret").run().await.unwrap_err();
    assert!(err.to_string().contains("blocked IP address"));
}

#[test]
fn ssrf_public_ip_allowed() {
    // Should not panic at construction time
    let _ = Http::get("http://8.8.8.8/dns");
}

#[test]
fn ssrf_hostname_allowed() {
    // Hostnames are not blocked at URL parse time (would need DNS)
    let _ = Http::get("https://example.com/api");
}

#[tokio::test]
async fn ssrf_172_16_blocked() {
    let err = Http::get("http://172.16.0.1/internal")
        .run()
        .await
        .unwrap_err();
    assert!(err.to_string().contains("blocked IP address"));
}

#[tokio::test]
async fn ssrf_192_168_blocked() {
    let err = Http::get("http://192.168.1.1/admin")
        .run()
        .await
        .unwrap_err();
    assert!(err.to_string().contains("blocked IP address"));
}

#[tokio::test]
async fn ssrf_unspecified_blocked() {
    let err = Http::get("http://0.0.0.0/").run().await.unwrap_err();
    assert!(err.to_string().contains("blocked IP address"));
}

#[tokio::test]
async fn ssrf_broadcast_blocked() {
    let err = Http::get("http://255.255.255.255/")
        .run()
        .await
        .unwrap_err();
    assert!(err.to_string().contains("blocked IP address"));
}

#[tokio::test]
async fn ssrf_localhost_with_port_blocked() {
    let err = Http::get("http://127.0.0.1:8080/secret")
        .run()
        .await
        .unwrap_err();
    assert!(err.to_string().contains("blocked IP address"));
}

/// Answers `200 ok` to one request on 127.0.0.1: a real internal service the guard
/// must keep out of reach.
async fn serve_once_ok() -> (u16, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 1024];
        let _ = socket.read(&mut buf).await.unwrap();
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
            .await
            .unwrap();
        socket.shutdown().await.unwrap();
    });
    (port, server)
}

#[tokio::test]
async fn ssrf_name_resolving_to_loopback_blocked() {
    let (port, server) = serve_once_ok().await;
    let err = Http::get(&format!("http://localhost:{port}/secret"))
        .timeout(Duration::from_secs(5))
        .run()
        .await
        .unwrap_err();
    server.abort();
    assert!(matches!(err, OperationError::Http { status: None, .. }));
    assert!(
        err.to_string()
            .contains("URL host localhost resolves to a blocked IP address"),
        "{err}"
    );
}

#[tokio::test]
async fn ssrf_ipv4_mapped_metadata_blocked() {
    let err = Http::get("http://[::ffff:169.254.169.254]/latest/meta-data/")
        .timeout(Duration::from_secs(2))
        .run()
        .await
        .unwrap_err();
    assert!(err.to_string().contains("blocked IP address"), "{err}");
}

#[tokio::test]
async fn ssrf_decimal_loopback_blocked() {
    let err = Http::get("http://2130706433/")
        .timeout(Duration::from_secs(2))
        .run()
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("blocked IP address (127.0.0.1)"),
        "{err}"
    );
}

#[tokio::test]
async fn allow_host_reaches_internal_server() {
    let (port, server) = serve_once_ok().await;
    let output = Http::get(&format!("http://localhost:{port}/health"))
        .allow_host("localhost")
        .timeout(Duration::from_secs(5))
        .run()
        .await
        .unwrap();
    assert_eq!(output.status(), 200);
    assert_eq!(output.body(), "ok");
    server.await.unwrap();
}

#[tokio::test]
async fn allow_host_does_not_allow_other_hosts() {
    let (port, server) = serve_once_ok().await;
    let err = Http::get(&format!("http://127.0.0.1:{port}/secret"))
        .allow_host("localhost")
        .timeout(Duration::from_secs(5))
        .run()
        .await
        .unwrap_err();
    server.abort();
    assert!(err.to_string().contains("blocked IP address"), "{err}");
}

#[test]
fn url_trimming_stores_trimmed() {
    let http = Http::get("  https://example.com  ");
    assert_eq!(http.url, "https://example.com");
}

#[test]
fn text_body_builder() {
    let http = Http::post("https://x.com").text("hello body");
    assert!(matches!(http.body, Some(HttpBody::Text(ref s)) if s == "hello body"));
}

#[test]
fn json_body_builder_stores_value() {
    let http = Http::post("https://x.com").json(serde_json::json!({"k": "v"}));
    assert!(matches!(http.body, Some(HttpBody::Json(_))));
}

#[test]
fn max_response_size_builder() {
    let http = Http::get("https://x.com").max_response_size(1024);
    assert_eq!(http.max_response_size, 1024);
}

#[test]
fn dry_run_builder_stores_flag() {
    let http = Http::get("https://x.com").dry_run(true);
    assert_eq!(http.dry_run, Some(true));
}

#[test]
fn retry_builder_stores_policy() {
    let http = Http::get("https://x.com").retry(3);
    assert!(http.retry_policy.is_some());
    assert_eq!(http.retry_policy.unwrap().max_retries(), 3);
}

#[test]
fn retry_policy_builder_stores_custom_policy() {
    let policy = RetryPolicy::new(5)
        .backoff(Duration::from_secs(1))
        .multiplier(3.0);
    let http = Http::get("https://x.com").retry_policy(policy);
    let p = http.retry_policy.unwrap();
    assert_eq!(p.max_retries(), 5);
    assert_eq!(p.initial_backoff, Duration::from_secs(1));
}

#[test]
fn no_retry_by_default() {
    let http = Http::get("https://x.com");
    assert!(http.retry_policy.is_none());
}

#[test]
fn http_output_accessors() {
    let mut headers = HashMap::new();
    headers.insert("content-type".to_string(), "text/plain".to_string());
    let output = HttpOutput {
        status: 201,
        headers,
        body: "hello".to_string(),
        duration_ms: 42,
    };
    assert_eq!(output.status(), 201);
    assert_eq!(output.body(), "hello");
    assert_eq!(output.duration_ms(), 42);
    assert_eq!(output.headers().get("content-type").unwrap(), "text/plain");
}

#[tokio::test]
async fn ssrf_userinfo_in_url_blocked() {
    let err = Http::get("http://user:pass@127.0.0.1/secret")
        .run()
        .await
        .unwrap_err();
    assert!(err.to_string().contains("blocked IP address"));
}

#[test]
fn blocked_literal_with_userinfo_detects_blocked_ip() {
    let url = Url::parse("http://admin:secret@10.0.0.1/path").unwrap();
    let result = ssrf::blocked_literal(&url);
    assert!(result.is_some());
    assert!(result.unwrap().to_string().contains("blocked IP address"));
}

#[test]
fn blocked_literal_public_ip_with_userinfo_allowed() {
    let url = Url::parse("http://user:pass@8.8.8.8/dns").unwrap();
    assert!(ssrf::blocked_literal(&url).is_none());
}

#[tokio::test]
async fn no_redirect_returns_3xx_status() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let response =
            "HTTP/1.1 302 Found\r\nLocation: http://10.0.0.1/evil\r\nContent-Length: 0\r\n\r\n";
        socket.write_all(response.as_bytes()).await.unwrap();
        socket.shutdown().await.unwrap();
    });

    let url = format!("http://localhost:{port}/test");

    let output = Http::get(&url)
        .allow_host("localhost")
        .timeout(Duration::from_secs(5))
        .run()
        .await
        .unwrap();

    assert_eq!(output.status(), 302);

    server.await.unwrap();
}

#[tokio::test]
async fn streaming_body_size_check_aborts_over_limit() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let body = "x".repeat(2048);
        let response = format!(
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{}\r\n0\r\n\r\n",
            body.len(),
            body,
        );
        socket.write_all(response.as_bytes()).await.unwrap();
        socket.shutdown().await.unwrap();
    });

    let url = format!("http://localhost:{port}/big");

    let result = Http::new(Method::GET, &url)
        .allow_host("localhost")
        .max_response_size(1024)
        .timeout(Duration::from_secs(5))
        .run()
        .await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("response body too large"));

    server.await.unwrap();
}
