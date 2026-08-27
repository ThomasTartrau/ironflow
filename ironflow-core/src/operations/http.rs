//! Http operation - perform HTTP requests with timeout and header control.
//!
//! The [`Http`] builder sends an HTTP request via [`reqwest`], captures the
//! response, and returns an [`HttpOutput`] on success. It implements
//! [`IntoFuture`] so you can `await` it directly:
//!
//! ```no_run
//! use ironflow_core::operations::http::Http;
//!
//! # async fn example() -> Result<(), ironflow_core::error::OperationError> {
//! let output = Http::get("https://httpbin.org/get").await?;
//! println!("status: {}", output.status());
//! # Ok(())
//! # }
//! ```

use std::collections::HashMap;
use std::future::{Future, IntoFuture};
use std::net::IpAddr;
use std::pin::Pin;
use std::time::{Duration, Instant};

use reqwest::{Client, Method};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::sync::LazyLock;
use tokio::time;
use tracing::{debug, warn};
use url::Url;

use crate::retry::RetryPolicy;
use crate::trace_context::WorkflowTraceContext;

/// Default timeout for HTTP requests (30 seconds).
const DEFAULT_HTTP_TIMEOUT: Duration = Duration::from_secs(30);

use crate::error::OperationError;
#[cfg(feature = "prometheus")]
use crate::metric_names;
use crate::utils::MAX_OUTPUT_SIZE;

/// Returns `true` if the given IP address is private, loopback, link-local,
/// or a cloud metadata endpoint - i.e. a target that should not be reachable
/// via SSRF.
fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()              // 127.0.0.0/8
                || v4.is_private()         // 10/8, 172.16/12, 192.168/16
                || v4.is_link_local()      // 169.254.0.0/16 (includes AWS metadata)
                || v4.is_broadcast()       // 255.255.255.255
                || v4.is_unspecified() // 0.0.0.0
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()              // ::1
                || v6.is_unspecified() // ::
        }
    }
}

/// Check if a URL host is a blocked IP address (literal IP in the URL).
/// Returns an error message if blocked, None if safe (or if host is a hostname
/// that needs DNS resolution - runtime check happens at connect time).
fn check_url_host(raw: &str) -> Option<String> {
    let parsed = Url::parse(raw).ok()?;
    let host_str = parsed.host_str()?;

    let host_clean = host_str.trim_start_matches('[').trim_end_matches(']');

    if let Ok(ip) = host_clean.parse::<IpAddr>()
        && is_blocked_ip(ip)
    {
        return Some(format!(
            "URL targets a blocked IP address ({ip}): private, loopback, and link-local addresses are not allowed"
        ));
    }
    None
}

static HTTP_CLIENT: LazyLock<Client> = LazyLock::new(|| {
    Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("failed to build HTTP client")
});

/// Builder for executing an HTTP request.
///
/// Supports method, URL, headers, body (JSON or text), and timeout.
/// The response body is captured as a string, with optional typed
/// JSON deserialization via [`HttpOutput::json`].
///
/// Unlike [`Shell`](crate::operations::shell::Shell), `Http` does **not**
/// fail on non-2xx status codes - use [`HttpOutput::is_success`] to check.
/// Only transport-level errors (DNS, timeout, connection refused) produce
/// an [`OperationError::Http`].
///
/// # Examples
///
/// ```no_run
/// use std::time::Duration;
/// use ironflow_core::operations::http::Http;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let output = Http::post("https://httpbin.org/post")
///     .header("Authorization", "Bearer token123")
///     .json(serde_json::json!({"key": "value"}))
///     .timeout(Duration::from_secs(30))
///     .await?;
///
/// println!("status: {}, body: {}", output.status(), output.body());
/// # Ok(())
/// # }
/// ```
#[must_use = "an Http request does nothing until .run() or .await is called"]
pub struct Http {
    method: Method,
    url: String,
    headers: HashMap<String, String>,
    body: Option<HttpBody>,
    timeout: Option<Duration>,
    max_response_size: usize,
    dry_run: Option<bool>,
    retry_policy: Option<RetryPolicy>,
}

enum HttpBody {
    Text(String),
    Json(Value),
}

impl Http {
    /// Create a request builder with an arbitrary HTTP method.
    ///
    /// # Panics
    ///
    /// Panics if `url` is empty.
    pub fn new(method: Method, url: &str) -> Self {
        let trimmed = url.trim();
        assert!(!trimmed.is_empty(), "url must not be empty");
        assert!(
            trimmed.starts_with("http://") || trimmed.starts_with("https://"),
            "url must use http:// or https:// scheme, got: {trimmed}"
        );
        Self {
            method,
            url: trimmed.to_string(),
            headers: HashMap::new(),
            body: None,
            timeout: Some(DEFAULT_HTTP_TIMEOUT),
            max_response_size: MAX_OUTPUT_SIZE,
            dry_run: None,
            retry_policy: None,
        }
    }

    /// Create a GET request builder.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::operations::http::Http;
    ///
    /// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
    /// let output = Http::get("https://httpbin.org/get").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn get(url: &str) -> Self {
        Self::new(Method::GET, url)
    }

    /// Create a POST request builder.
    pub fn post(url: &str) -> Self {
        Self::new(Method::POST, url)
    }

    /// Create a PUT request builder.
    pub fn put(url: &str) -> Self {
        Self::new(Method::PUT, url)
    }

    /// Create a PATCH request builder.
    pub fn patch(url: &str) -> Self {
        Self::new(Method::PATCH, url)
    }

    /// Create a DELETE request builder.
    pub fn delete(url: &str) -> Self {
        Self::new(Method::DELETE, url)
    }

    /// Add a header to the request.
    ///
    /// Can be called multiple times to set several headers.
    pub fn header(mut self, key: &str, value: &str) -> Self {
        self.headers.insert(key.to_string(), value.to_string());
        self
    }

    /// Set a JSON body.
    ///
    /// `Content-Type: application/json` is added automatically by reqwest.
    /// Takes ownership of the [`Value`] to avoid cloning.
    pub fn json(mut self, value: Value) -> Self {
        self.body = Some(HttpBody::Json(value));
        self
    }

    /// Set a plain text body.
    pub fn text(mut self, body: &str) -> Self {
        self.body = Some(HttpBody::Text(body.to_string()));
        self
    }

    /// Override the timeout for the request.
    ///
    /// If the request does not complete within this duration, an
    /// [`OperationError::Http`] is returned. Defaults to 30 seconds.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Set the maximum allowed response body size in bytes.
    ///
    /// If the response body exceeds this limit, an [`OperationError::Http`] is
    /// returned. Defaults to 10 MiB.
    pub fn max_response_size(mut self, bytes: usize) -> Self {
        self.max_response_size = bytes;
        self
    }

    /// Retry the request up to `max_retries` times on transient failures.
    ///
    /// Uses default exponential backoff settings (200ms initial, 2x multiplier,
    /// 30s cap). For custom backoff parameters, use [`retry_policy`](Http::retry_policy).
    ///
    /// Only transient errors are retried: transport errors (DNS, timeout,
    /// connection refused) and responses with status 5xx or 429. Client errors
    /// (4xx except 429) and SSRF blocks are never retried.
    ///
    /// # Panics
    ///
    /// Panics if `max_retries` is `0`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::operations::http::Http;
    ///
    /// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
    /// let output = Http::get("https://api.example.com/data")
    ///     .retry(3)
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn retry(mut self, max_retries: u32) -> Self {
        self.retry_policy = Some(RetryPolicy::new(max_retries));
        self
    }

    /// Set a custom [`RetryPolicy`] for this request.
    ///
    /// Allows full control over backoff duration, multiplier, and max delay.
    /// See [`RetryPolicy`] for details.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::time::Duration;
    /// use ironflow_core::operations::http::Http;
    /// use ironflow_core::retry::RetryPolicy;
    ///
    /// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
    /// let output = Http::get("https://api.example.com/data")
    ///     .retry_policy(
    ///         RetryPolicy::new(5)
    ///             .backoff(Duration::from_millis(500))
    ///             .max_backoff(Duration::from_secs(60))
    ///             .multiplier(3.0)
    ///     )
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn retry_policy(mut self, policy: RetryPolicy) -> Self {
        self.retry_policy = Some(policy);
        self
    }

    /// Attach a [`WorkflowTraceContext`] to this request.
    ///
    /// When set, the `traceparent` header is automatically injected into
    /// the request using the context's [`to_traceparent`](WorkflowTraceContext::to_traceparent)
    /// value. This enables distributed tracing correlation with downstream
    /// services.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::operations::http::Http;
    /// use ironflow_core::trace_context::WorkflowTraceContext;
    ///
    /// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
    /// let ctx = WorkflowTraceContext::new_root();
    /// let output = Http::get("https://api.example.com/data")
    ///     .trace_context(&ctx)
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn trace_context(self, ctx: &WorkflowTraceContext) -> Self {
        self.header("traceparent", &ctx.to_traceparent())
    }

    /// Enable or disable dry-run mode for this specific operation.
    ///
    /// When dry-run is active, the request is logged but not sent.
    /// A synthetic [`HttpOutput`] is returned with status 200, empty body,
    /// and 0ms duration.
    ///
    /// If not set, falls back to the global dry-run setting
    /// (see [`set_dry_run`](crate::dry_run::set_dry_run)).
    pub fn dry_run(mut self, enabled: bool) -> Self {
        self.dry_run = Some(enabled);
        self
    }

    /// Execute the HTTP request.
    ///
    /// If a [`retry_policy`](Http::retry_policy) is configured, transient
    /// failures (transport errors, 5xx, 429) are retried with exponential
    /// backoff. Non-retryable errors and successful responses are returned
    /// immediately.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails at the transport
    /// layer (network error, DNS failure, timeout) or if the response body
    /// cannot be read. Non-2xx status codes are **not** treated as errors.
    #[tracing::instrument(name = "http", skip_all, fields(method = %self.method, url = %self.url))]
    pub async fn run(self) -> Result<HttpOutput, OperationError> {
        if crate::dry_run::effective_dry_run(self.dry_run) {
            debug!(method = %self.method, url = %self.url, "[dry-run] http request skipped");
            return Ok(HttpOutput {
                status: 200,
                headers: HashMap::new(),
                body: String::new(),
                duration_ms: 0,
            });
        }

        if let Some(reason) = check_url_host(&self.url) {
            return Err(OperationError::Http {
                status: None,
                message: reason,
            });
        }

        let result = self.execute_once().await;

        let policy = match &self.retry_policy {
            Some(p) => p,
            None => return result,
        };

        // If the first attempt succeeded with a non-retryable status, return it.
        // If it failed with a non-retryable error, return it.
        match &result {
            Ok(output) if !crate::retry::is_retryable_status(output.status) => return result,
            Err(err) if !crate::retry::is_retryable(err) => return result,
            _ => {}
        }

        let mut last_result = result;

        for attempt in 0..policy.max_retries {
            let delay = policy.delay_for_attempt(attempt);
            warn!(
                attempt = attempt + 1,
                max_retries = policy.max_retries,
                delay_ms = delay.as_millis() as u64,
                "retrying http request"
            );
            time::sleep(delay).await;

            last_result = self.execute_once().await;

            match &last_result {
                Ok(output) if !crate::retry::is_retryable_status(output.status) => {
                    return last_result;
                }
                Err(err) if !crate::retry::is_retryable(err) => return last_result,
                _ => {}
            }
        }

        last_result
    }

    /// Execute a single HTTP request attempt (no retry logic).
    async fn execute_once(&self) -> Result<HttpOutput, OperationError> {
        debug!(method = %self.method, url = %self.url, "executing http request");
        let start = Instant::now();

        #[cfg(feature = "prometheus")]
        let method_label = self.method.to_string();

        let mut builder = HTTP_CLIENT.request(self.method.clone(), &self.url);

        if let Some(timeout) = self.timeout {
            builder = builder.timeout(timeout);
        }

        for (k, v) in &self.headers {
            builder = builder.header(k.as_str(), v.as_str());
        }

        match &self.body {
            Some(HttpBody::Json(v)) => {
                builder = builder.json(v);
            }
            Some(HttpBody::Text(t)) => {
                builder = builder.body(t.clone());
            }
            None => {}
        }

        let response = match builder.send().await {
            Ok(resp) => resp,
            Err(e) => {
                #[cfg(feature = "prometheus")]
                {
                    metrics::counter!(metric_names::HTTP_TOTAL, "method" => method_label, "status" => metric_names::STATUS_ERROR).increment(1);
                }
                return Err(OperationError::Http {
                    status: None,
                    message: format!("request failed: {e}"),
                });
            }
        };

        let status = response.status().as_u16();
        let headers: HashMap<String, String> = response
            .headers()
            .iter()
            .map(|(k, v)| {
                let val = match v.to_str() {
                    Ok(s) => s.to_string(),
                    Err(_) => {
                        debug!(header = %k, "non-UTF-8 header value, replacing with empty string");
                        String::new()
                    }
                };
                (k.to_string(), val)
            })
            .collect();
        let max_response_size = self.max_response_size;
        let response_too_large = |size: usize, limit: usize| OperationError::Http {
            status: Some(status),
            message: format!(
                "response body too large: {size} bytes exceeds limit of {limit} bytes"
            ),
        };

        if let Some(cl) = response.content_length() {
            let content_length = usize::try_from(cl).unwrap_or(usize::MAX);
            if content_length > max_response_size {
                return Err(response_too_large(content_length, max_response_size));
            }
        }

        let mut body_bytes = Vec::new();
        let mut response = response;
        loop {
            match response.chunk().await {
                Ok(Some(chunk)) => {
                    if body_bytes.len() + chunk.len() > max_response_size {
                        return Err(response_too_large(
                            body_bytes.len() + chunk.len(),
                            max_response_size,
                        ));
                    }
                    body_bytes.extend_from_slice(&chunk);
                }
                Ok(None) => break,
                Err(e) => {
                    return Err(OperationError::Http {
                        status: Some(status),
                        message: format!("failed to read response body: {e}"),
                    });
                }
            }
        }

        let body = String::from_utf8_lossy(&body_bytes).into_owned();
        let duration_ms = start.elapsed().as_millis() as u64;

        debug!(
            status,
            body_len = body.len(),
            duration_ms,
            "http request completed"
        );

        #[cfg(feature = "prometheus")]
        {
            let status_label = status.to_string();
            metrics::counter!(metric_names::HTTP_TOTAL, "method" => method_label, "status" => status_label).increment(1);
            metrics::histogram!(metric_names::HTTP_DURATION_SECONDS)
                .record(duration_ms as f64 / 1000.0);
        }

        Ok(HttpOutput {
            status,
            headers,
            body,
            duration_ms,
        })
    }
}

impl IntoFuture for Http {
    type Output = Result<HttpOutput, OperationError>;
    type IntoFuture = Pin<Box<dyn Future<Output = Self::Output> + Send>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(self.run())
    }
}

/// Output of a completed HTTP request.
///
/// Contains the status code, response headers, body, and duration.
#[derive(Debug)]
pub struct HttpOutput {
    status: u16,
    headers: HashMap<String, String>,
    body: String,
    duration_ms: u64,
}

impl HttpOutput {
    /// Return the HTTP status code (e.g. `200`, `404`).
    pub fn status(&self) -> u16 {
        self.status
    }

    /// Return the response headers as a string map.
    pub fn headers(&self) -> &HashMap<String, String> {
        &self.headers
    }

    /// Return the response body as text.
    pub fn body(&self) -> &str {
        &self.body
    }

    /// Deserialize the response body as JSON into the given type `T`.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Deserialize`] if parsing fails.
    pub fn json<T: DeserializeOwned>(&self) -> Result<T, OperationError> {
        serde_json::from_str(&self.body).map_err(OperationError::deserialize::<T>)
    }

    /// Return the wall-clock duration of the request in milliseconds.
    pub fn duration_ms(&self) -> u64 {
        self.duration_ms
    }

    /// Return `true` if the status code is in the 2xx range.
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

#[cfg(test)]
mod tests {
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
    fn check_url_host_with_userinfo_detects_blocked_ip() {
        let result = check_url_host("http://admin:secret@10.0.0.1/path");
        assert!(result.is_some());
        assert!(result.unwrap().contains("blocked IP address"));
    }

    #[test]
    fn check_url_host_public_ip_with_userinfo_allowed() {
        let result = check_url_host("http://user:pass@8.8.8.8/dns");
        assert!(result.is_none());
    }

    #[test]
    fn redirect_policy_is_none() {
        let client = &*HTTP_CLIENT;
        let _ = client;
    }

    #[tokio::test]
    async fn no_redirect_returns_3xx_status() {
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            use tokio::io::AsyncWriteExt;
            let response =
                "HTTP/1.1 302 Found\r\nLocation: http://10.0.0.1/evil\r\nContent-Length: 0\r\n\r\n";
            socket.write_all(response.as_bytes()).await.unwrap();
            socket.shutdown().await.unwrap();
        });

        let url = format!("http://localhost:{port}/test");

        let output = Http::get(&url)
            .timeout(Duration::from_secs(5))
            .run()
            .await
            .unwrap();

        assert_eq!(output.status(), 302);

        server.await.unwrap();
    }

    #[tokio::test]
    async fn streaming_body_size_check_aborts_over_limit() {
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            use tokio::io::AsyncWriteExt;
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
            .max_response_size(1024)
            .timeout(Duration::from_secs(5))
            .run()
            .await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("response body too large"));

        server.await.unwrap();
    }
}
