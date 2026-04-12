//! Retry with exponential backoff for outbound HTTP deliveries.
//!
//! Shared by all built-in subscribers ([`WebhookSubscriber`](super::WebhookSubscriber),
//! [`BetterStackSubscriber`](super::BetterStackSubscriber), etc.) so that
//! retry logic is defined once.

use std::fmt::Display;
use std::time::Duration;

use reqwest::{Client, RequestBuilder, Response, StatusCode};
use tokio::time::sleep;
use tracing::{error, info, warn};

/// Configuration for retry behaviour on outbound HTTP calls.
///
/// All built-in subscribers share this configuration. Construct with
/// [`RetryConfig::new`] or use [`RetryConfig::default`] for sensible
/// defaults (3 attempts, 5 s timeout, 500 ms base backoff).
///
/// # Examples
///
/// ```
/// use ironflow_engine::notify::RetryConfig;
///
/// let config = RetryConfig::default();
/// assert_eq!(config.max_retries(), 3);
/// ```
#[derive(Debug, Clone)]
pub struct RetryConfig {
    max_retries: u32,
    timeout: Duration,
    base_backoff: Duration,
}

impl RetryConfig {
    /// Default timeout for outbound HTTP calls.
    const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

    /// Default maximum number of retry attempts.
    const DEFAULT_MAX_RETRIES: u32 = 3;

    /// Default base delay for exponential backoff (doubled each retry).
    const DEFAULT_BASE_BACKOFF: Duration = Duration::from_millis(500);

    /// Create a new retry configuration.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    /// use ironflow_engine::notify::RetryConfig;
    ///
    /// let config = RetryConfig::new(5, Duration::from_secs(10), Duration::from_secs(1));
    /// assert_eq!(config.max_retries(), 5);
    /// assert_eq!(config.timeout(), Duration::from_secs(10));
    /// assert_eq!(config.base_backoff(), Duration::from_secs(1));
    /// ```
    pub fn new(max_retries: u32, timeout: Duration, base_backoff: Duration) -> Self {
        Self {
            max_retries,
            timeout,
            base_backoff,
        }
    }

    /// Maximum number of retry attempts.
    pub fn max_retries(&self) -> u32 {
        self.max_retries
    }

    /// Timeout per HTTP request.
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Base delay for exponential backoff (doubled each retry).
    pub fn base_backoff(&self) -> Duration {
        self.base_backoff
    }

    /// Build an HTTP client configured with this timeout.
    ///
    /// # Panics
    ///
    /// Panics if the TLS backend is unavailable.
    pub fn build_client(&self) -> Client {
        Client::builder()
            .timeout(self.timeout)
            .build()
            .expect("failed to build HTTP client")
    }
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: Self::DEFAULT_MAX_RETRIES,
            timeout: Self::DEFAULT_TIMEOUT,
            base_backoff: Self::DEFAULT_BASE_BACKOFF,
        }
    }
}

/// Predicate that decides whether an HTTP response counts as success.
///
/// The default for webhooks is `Response::status().is_success()` (2xx),
/// while BetterStack expects exactly `202 Accepted`.
pub type SuccessPredicate = fn(&Response) -> bool;

/// Returns `true` when the response status is 2xx.
pub fn is_success_2xx(response: &Response) -> bool {
    response.status().is_success()
}

/// Returns `true` when the response status is exactly `202 Accepted`.
pub fn is_accepted_202(response: &Response) -> bool {
    response.status() == StatusCode::ACCEPTED
}

/// Execute an HTTP request with retry and exponential backoff.
///
/// Calls `build_request` before each attempt to produce a fresh
/// [`RequestBuilder`] (request builders are consumed on send).
/// `is_success` determines whether a given response is acceptable.
///
/// `subscriber_name` and `context` are used only for structured logging.
///
/// # Examples
///
/// ```no_run
/// use ironflow_engine::notify::{RetryConfig, deliver_with_retry, is_success_2xx};
/// use reqwest::Client;
///
/// # async fn example() {
/// let config = RetryConfig::default();
/// let client = config.build_client();
/// let url = "https://example.com/hook";
///
/// deliver_with_retry(
///     &config,
///     || client.post(url).body("{}"),
///     is_success_2xx,
///     "webhook",
///     url,
/// ).await;
/// # }
/// ```
pub async fn deliver_with_retry(
    config: &RetryConfig,
    build_request: impl Fn() -> RequestBuilder,
    is_success: SuccessPredicate,
    subscriber_name: &str,
    context: &(impl Display + ?Sized),
) {
    for attempt in 0..config.max_retries {
        let result = build_request().send().await;

        match result {
            Ok(resp) if is_success(&resp) => {
                info!(
                    subscriber = subscriber_name,
                    context = %context,
                    "delivery succeeded"
                );
                return;
            }
            Ok(resp) => {
                let status = resp.status();
                log_retry_or_fail(
                    config,
                    attempt,
                    subscriber_name,
                    context,
                    &format!("HTTP {status}"),
                );
            }
            Err(err) => {
                log_retry_or_fail(config, attempt, subscriber_name, context, &err.to_string());
            }
        }

        if attempt + 1 < config.max_retries {
            let delay = config.base_backoff * 2u32.pow(attempt);
            sleep(delay).await;
        }
    }
}

fn log_retry_or_fail(
    config: &RetryConfig,
    attempt: u32,
    subscriber_name: &str,
    context: &(impl Display + ?Sized),
    err_msg: &str,
) {
    let remaining = config.max_retries - attempt - 1;
    if remaining > 0 {
        warn!(
            subscriber = subscriber_name,
            context = %context,
            attempt = attempt + 1,
            remaining,
            error = %err_msg,
            "delivery failed, retrying"
        );
    } else {
        error!(
            subscriber = subscriber_name,
            context = %context,
            error = %err_msg,
            "delivery failed after all retries"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_values() {
        let config = RetryConfig::default();
        assert_eq!(config.max_retries(), 3);
        assert_eq!(config.timeout(), Duration::from_secs(5));
        assert_eq!(config.base_backoff(), Duration::from_millis(500));
    }

    #[test]
    fn custom_config_values() {
        let config = RetryConfig::new(5, Duration::from_secs(10), Duration::from_secs(1));
        assert_eq!(config.max_retries(), 5);
        assert_eq!(config.timeout(), Duration::from_secs(10));
        assert_eq!(config.base_backoff(), Duration::from_secs(1));
    }

    #[test]
    fn build_client_succeeds() {
        let config = RetryConfig::default();
        let _client = config.build_client();
    }

    use axum::http::Response as HttpResponse;

    #[test]
    fn is_success_2xx_predicate() {
        let response = HttpResponse::builder().status(200).body("").unwrap();
        let reqwest_resp = Response::from(response);
        assert!(is_success_2xx(&reqwest_resp));
    }

    #[test]
    fn is_success_2xx_rejects_4xx() {
        let response = HttpResponse::builder().status(400).body("").unwrap();
        let reqwest_resp = Response::from(response);
        assert!(!is_success_2xx(&reqwest_resp));
    }

    #[test]
    fn is_accepted_202_predicate() {
        let response = HttpResponse::builder().status(202).body("").unwrap();
        let reqwest_resp = Response::from(response);
        assert!(is_accepted_202(&reqwest_resp));
    }

    #[test]
    fn is_accepted_202_rejects_200() {
        let response = HttpResponse::builder().status(200).body("").unwrap();
        let reqwest_resp = Response::from(response);
        assert!(!is_accepted_202(&reqwest_resp));
    }

    #[tokio::test]
    async fn deliver_succeeds_on_first_try() {
        use axum::Router;
        use axum::http::StatusCode;
        use axum::routing::post;
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let app = Router::new().route("/", post(|| async { StatusCode::OK }));
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let config = RetryConfig::default();
        let client = config.build_client();
        let url = format!("http://{}", addr);

        deliver_with_retry(
            &config,
            || client.post(&url).body("{}"),
            is_success_2xx,
            "test",
            &url,
        )
        .await;
    }

    #[tokio::test]
    async fn deliver_retries_on_server_error() {
        use axum::Router;
        use axum::http::StatusCode;
        use axum::routing::post;
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU32, Ordering};
        use tokio::net::TcpListener;

        let call_count = Arc::new(AtomicU32::new(0));
        let count = call_count.clone();

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let app = Router::new().route(
            "/",
            post(move || {
                let count = count.clone();
                async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    StatusCode::INTERNAL_SERVER_ERROR
                }
            }),
        );
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let config = RetryConfig::new(3, Duration::from_secs(5), Duration::from_millis(10));
        let client = config.build_client();
        let url = format!("http://{}", addr);

        deliver_with_retry(
            &config,
            || client.post(&url).body("{}"),
            is_success_2xx,
            "test",
            &url,
        )
        .await;

        assert_eq!(call_count.load(Ordering::SeqCst), 3);
    }
}
