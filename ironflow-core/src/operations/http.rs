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
use std::env::{self, VarError};
use std::future::{Future, IntoFuture};
use std::pin::Pin;
use std::time::{Duration, Instant};

use reqwest::redirect::Policy;
use reqwest::{Client, Method};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::sync::LazyLock;
use tokio::time;
use tracing::{debug, warn};
use url::Url;

use crate::retry::RetryPolicy;
use crate::ssrf::{self, AllowedHosts, GuardedResolver};
use crate::trace_context::WorkflowTraceContext;

/// Default timeout for HTTP requests (30 seconds).
const DEFAULT_HTTP_TIMEOUT: Duration = Duration::from_secs(30);

use crate::error::OperationError;
#[cfg(feature = "prometheus")]
use crate::metric_names;
use crate::utils::MAX_OUTPUT_SIZE;

/// Environment variable listing, comma-separated, the hosts every [`Http`] request of
/// the deployment may reach even when they are internal.
const ALLOWED_HOSTS_ENV: &str = "IRONFLOW_HTTP_ALLOWED_HOSTS";

static ENV_ALLOWED_HOSTS: LazyLock<AllowedHosts> =
    LazyLock::new(|| match env::var(ALLOWED_HOSTS_ENV) {
        Ok(list) => AllowedHosts::parse_list(&list),
        Err(VarError::NotPresent) => AllowedHosts::default(),
        Err(err) => {
            warn!(error = %err, "{ALLOWED_HOSTS_ENV} ignored: no internal host is allowed");
            AllowedHosts::default()
        }
    });

/// Client for hosts that may be internal: no SSRF guard, environment proxies honored.
static HTTP_CLIENT: LazyLock<Client> = LazyLock::new(|| {
    Client::builder()
        .redirect(Policy::none())
        .build()
        .expect("failed to build HTTP client")
});

/// Client for every other host. Proxies are ignored: a proxy resolves the target
/// itself, out of reach of [`GuardedResolver`].
static GUARDED_HTTP_CLIENT: LazyLock<Client> = LazyLock::new(|| {
    Client::builder()
        .redirect(Policy::none())
        .no_proxy()
        .dns_resolver(GuardedResolver::default())
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
    allowed_hosts: AllowedHosts,
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
            allowed_hosts: AllowedHosts::default(),
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

    /// Allow this request to reach `host` even when it is, or resolves to, a private,
    /// loopback, link-local or cloud metadata address.
    ///
    /// Without it, such a target fails with [`OperationError::Http`] before anything is
    /// sent. Pass the host as it appears in the URL (`"billing.internal"`, `"10.0.0.5"`,
    /// `"::1"`); the match ignores case and IPv6 brackets. Hosts allowed for the whole
    /// deployment go in the `IRONFLOW_HTTP_ALLOWED_HOSTS` environment variable, a
    /// comma-separated list read once at the first request.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::operations::http::Http;
    ///
    /// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
    /// let output = Http::get("http://billing.internal:8080/invoices")
    ///     .allow_host("billing.internal")
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn allow_host(mut self, host: &str) -> Self {
        self.allowed_hosts.add(host);
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
    ///
    /// Also returns [`OperationError::Http`], before anything is sent and without
    /// retries, if the URL cannot be parsed, or if its host is, or resolves to, a
    /// private, loopback, link-local or cloud metadata address that
    /// [`allow_host`](Http::allow_host) or `IRONFLOW_HTTP_ALLOWED_HOSTS` does not allow.
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

        let url = Url::parse(&self.url).map_err(|e| OperationError::Http {
            status: None,
            message: format!("invalid URL {}: {e}", self.url),
        })?;
        let client = if self.allowed_hosts.contains_url_host(&url)
            || ENV_ALLOWED_HOSTS.contains_url_host(&url)
        {
            &*HTTP_CLIENT
        } else {
            ssrf::check_url(&url)
                .await
                .map_err(|blocked| OperationError::Http {
                    status: None,
                    message: blocked.to_string(),
                })?;
            &*GUARDED_HTTP_CLIENT
        };

        let result = self.execute_once(client).await;

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

            last_result = self.execute_once(client).await;

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
    async fn execute_once(&self, client: &Client) -> Result<HttpOutput, OperationError> {
        debug!(method = %self.method, url = %self.url, "executing http request");
        let start = Instant::now();

        #[cfg(feature = "prometheus")]
        let method_label = self.method.to_string();

        let mut builder = client.request(self.method.clone(), &self.url);

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
                    // A DNS answer that changed since `check_url` (rebinding) is refused
                    // by the resolver, deep in the source chain.
                    message: match ssrf::find_blocked(&e) {
                        Some(blocked) => blocked.to_string(),
                        None => format!("request failed: {e}"),
                    },
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
mod tests;
