//! [`WebhookSubscriber`] -- POSTs events as JSON to a URL.
//!
//! When a signing secret is configured, each outbound request includes an
//! `X-Signature-256` header containing the HMAC-SHA256 digest of the JSON
//! body, prefixed with `sha256=` (the same convention used by GitHub and
//! GitLab). Receivers can verify the signature to ensure the payload was
//! not tampered with in transit.

use hmac::{Hmac, Mac};
use reqwest::Client;
use sha2::Sha256;

use super::retry::{RetryConfig, deliver_with_retry, is_success_2xx};
use super::{Event, EventSubscriber, SubscriberFuture};

type HmacSha256 = Hmac<Sha256>;

/// Header name used for outbound HMAC-SHA256 signatures.
const SIGNATURE_HEADER: &str = "X-Signature-256";

/// Subscriber that POSTs the event as JSON to a webhook URL.
///
/// Retries failed deliveries with exponential backoff (up to 3 attempts,
/// 5 s timeout per attempt). The HTTP client is created once and reused.
///
/// When a signing secret is provided via [`WebhookSubscriber::with_signing_secret`],
/// every outbound request includes an `X-Signature-256` header whose value
/// is `sha256=<hex-encoded HMAC-SHA256>`, computed over the raw JSON body.
/// This lets the receiver verify payload authenticity.
///
/// Event type filtering is handled by the
/// [`EventPublisher`](super::EventPublisher) at subscription time -- this
/// subscriber receives only events that already passed the filter.
///
/// # Examples
///
/// ```no_run
/// use ironflow_engine::notify::{Event, EventPublisher, WebhookSubscriber};
///
/// let mut publisher = EventPublisher::new();
///
/// // Without signature
/// publisher.subscribe(
///     WebhookSubscriber::new("https://hooks.example.com/events"),
///     &[Event::RUN_STATUS_CHANGED, Event::STEP_FAILED],
/// );
///
/// // With HMAC-SHA256 signature
/// publisher.subscribe(
///     WebhookSubscriber::with_signing_secret(
///         "https://hooks.example.com/signed",
///         "my-webhook-secret",
///     ),
///     Event::ALL,
/// );
/// ```
pub struct WebhookSubscriber {
    url: String,
    signing_secret: Option<String>,
    client: Client,
    retry_config: RetryConfig,
}

impl WebhookSubscriber {
    /// Create a new webhook subscriber targeting the given URL.
    ///
    /// Uses the default [`RetryConfig`] (3 retries, 5 s timeout, 500 ms
    /// base backoff). No outbound signature is added.
    ///
    /// # Panics
    ///
    /// Panics if the HTTP client cannot be built (TLS backend unavailable).
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::notify::WebhookSubscriber;
    ///
    /// let subscriber = WebhookSubscriber::new("https://example.com/hook");
    /// assert_eq!(subscriber.url(), "https://example.com/hook");
    /// assert!(subscriber.signing_secret().is_none());
    /// ```
    pub fn new(url: &str) -> Self {
        Self::with_retry_config(url, RetryConfig::default())
    }

    /// Create a webhook subscriber with a custom retry configuration.
    ///
    /// No outbound signature is added.
    ///
    /// # Panics
    ///
    /// Panics if the HTTP client cannot be built (TLS backend unavailable).
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::notify::{RetryConfig, WebhookSubscriber};
    ///
    /// let config = RetryConfig::new(
    ///     5,
    ///     std::time::Duration::from_secs(10),
    ///     std::time::Duration::from_secs(1),
    /// );
    /// let subscriber = WebhookSubscriber::with_retry_config("https://example.com/hook", config);
    /// ```
    pub fn with_retry_config(url: &str, retry_config: RetryConfig) -> Self {
        Self::build(url, None, retry_config)
    }

    /// Create a webhook subscriber that signs outbound payloads with HMAC-SHA256.
    ///
    /// Each request will include an `X-Signature-256` header containing
    /// `sha256=<hex-encoded digest>`, computed over the raw JSON body.
    ///
    /// Uses the default [`RetryConfig`].
    ///
    /// # Panics
    ///
    /// Panics if the HTTP client cannot be built (TLS backend unavailable).
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::notify::WebhookSubscriber;
    ///
    /// let subscriber = WebhookSubscriber::with_signing_secret(
    ///     "https://example.com/hook",
    ///     "my-secret",
    /// );
    /// assert!(subscriber.signing_secret().is_some());
    /// ```
    pub fn with_signing_secret(url: &str, secret: &str) -> Self {
        Self::with_signing_secret_and_retry(url, secret, RetryConfig::default())
    }

    /// Create a webhook subscriber with HMAC-SHA256 signing and custom retry config.
    ///
    /// # Panics
    ///
    /// Panics if the HTTP client cannot be built (TLS backend unavailable).
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::notify::{RetryConfig, WebhookSubscriber};
    ///
    /// let config = RetryConfig::new(
    ///     5,
    ///     std::time::Duration::from_secs(10),
    ///     std::time::Duration::from_secs(1),
    /// );
    /// let subscriber = WebhookSubscriber::with_signing_secret_and_retry(
    ///     "https://example.com/hook",
    ///     "my-secret",
    ///     config,
    /// );
    /// ```
    pub fn with_signing_secret_and_retry(
        url: &str,
        secret: &str,
        retry_config: RetryConfig,
    ) -> Self {
        Self::build(url, Some(secret), retry_config)
    }

    fn build(url: &str, signing_secret: Option<&str>, retry_config: RetryConfig) -> Self {
        let client = retry_config.build_client();
        Self {
            url: url.to_string(),
            signing_secret: signing_secret.map(|s| s.to_string()),
            client,
            retry_config,
        }
    }

    /// Returns the target URL.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Returns the signing secret, if configured.
    pub fn signing_secret(&self) -> Option<&str> {
        self.signing_secret.as_deref()
    }

    /// Compute the `sha256=<hex>` signature string for a given body.
    fn compute_signature(secret: &str, body: &[u8]) -> String {
        let mut mac =
            HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key size");
        mac.update(body);
        format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
    }
}

impl EventSubscriber for WebhookSubscriber {
    fn name(&self) -> &str {
        "webhook"
    }

    fn handle<'a>(&'a self, event: &'a Event) -> SubscriberFuture<'a> {
        Box::pin(async move {
            let body = serde_json::to_vec(event).expect("Event is always serializable");
            let signature = self
                .signing_secret
                .as_deref()
                .map(|secret| Self::compute_signature(secret, &body));

            deliver_with_retry(
                &self.retry_config,
                || {
                    let mut req = self
                        .client
                        .post(&self.url)
                        .header("Content-Type", "application/json")
                        .body(body.clone());
                    if let Some(sig) = &signature {
                        req = req.header(SIGNATURE_HEADER, sig.as_str());
                    }
                    req
                },
                is_success_2xx,
                "webhook",
                &self.url,
            )
            .await;
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::Duration;

    use axum::Router;
    use axum::body::Bytes;
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::post;
    use chrono::Utc;
    use hmac::{Hmac, Mac};
    use ironflow_store::models::RunStatus;
    use rust_decimal::Decimal;
    use sha2::Sha256;
    use tokio::net::TcpListener;
    use tokio::sync::Mutex;
    use uuid::Uuid;

    use super::*;

    type HmacSha256 = Hmac<Sha256>;
    type CapturedRequest = Arc<Mutex<Option<(HeaderMap, Vec<u8>)>>>;

    fn compute_expected_hmac(secret: &[u8], body: &[u8]) -> String {
        let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC key rejected");
        mac.update(body);
        format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
    }

    #[test]
    fn url_accessor() {
        let sub = WebhookSubscriber::new("https://example.com/hook");
        assert_eq!(sub.url(), "https://example.com/hook");
    }

    #[test]
    fn name_is_webhook() {
        let sub = WebhookSubscriber::new("https://example.com");
        assert_eq!(sub.name(), "webhook");
    }

    #[test]
    fn no_signing_secret_by_default() {
        let sub = WebhookSubscriber::new("https://example.com/hook");
        assert!(sub.signing_secret().is_none());
    }

    #[test]
    fn with_signing_secret_stores_secret() {
        let sub = WebhookSubscriber::with_signing_secret("https://example.com/hook", "my-secret");
        assert_eq!(sub.signing_secret(), Some("my-secret"));
    }

    #[test]
    fn with_signing_secret_and_retry_stores_secret() {
        let config = RetryConfig::new(5, Duration::from_secs(10), Duration::from_secs(1));
        let sub = WebhookSubscriber::with_signing_secret_and_retry(
            "https://example.com/hook",
            "my-secret",
            config,
        );
        assert_eq!(sub.signing_secret(), Some("my-secret"));
        assert_eq!(sub.url(), "https://example.com/hook");
    }

    #[test]
    fn compute_signature_matches_hmac_sha256() {
        let secret = "test-secret";
        let body = b"{\"type\":\"run_created\"}";
        let sig = WebhookSubscriber::compute_signature(secret, body);
        let expected = compute_expected_hmac(secret.as_bytes(), body);
        assert_eq!(sig, expected);
    }

    #[test]
    fn compute_signature_empty_body() {
        let secret = "test-secret";
        let body = b"";
        let sig = WebhookSubscriber::compute_signature(secret, body);
        let expected = compute_expected_hmac(secret.as_bytes(), body);
        assert_eq!(sig, expected);
    }

    #[test]
    fn compute_signature_has_sha256_prefix() {
        let sig = WebhookSubscriber::compute_signature("secret", b"body");
        assert!(sig.starts_with("sha256="));
        assert_eq!(sig.len(), 7 + 64); // "sha256=" + 64 hex chars
    }

    #[test]
    fn compute_signature_rfc4231_test_vector() {
        // RFC 4231 Test Case 2: Key = "Jefe", Data = "what do ya want for nothing?"
        let key = "Jefe";
        let data = b"what do ya want for nothing?";
        let expected = "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843";

        let sig = WebhookSubscriber::compute_signature(key, data);
        assert_eq!(sig, format!("sha256={}", expected));
    }

    #[tokio::test]
    async fn unsigned_webhook_does_not_send_signature_header() {
        let received_headers: Arc<Mutex<Option<HeaderMap>>> = Arc::new(Mutex::new(None));
        let captured = received_headers.clone();

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let app = Router::new().route(
            "/",
            post(move |headers: HeaderMap, _body: Bytes| {
                let captured = captured.clone();
                async move {
                    *captured.lock().await = Some(headers);
                    StatusCode::OK
                }
            }),
        );
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let sub = WebhookSubscriber::new(&format!("http://{}", addr));
        let event = Event::RunCreated {
            run_id: Uuid::now_v7(),
            workflow_name: "deploy".to_string(),
            at: Utc::now(),
        };

        sub.handle(&event).await;

        let headers = received_headers.lock().await;
        let headers = headers.as_ref().expect("request was received");
        assert!(headers.get("X-Signature-256").is_none());
    }

    #[tokio::test]
    async fn signed_webhook_sends_valid_signature_header() {
        let secret = "webhook-secret-42";

        let received: CapturedRequest = Arc::new(Mutex::new(None));
        let captured = received.clone();

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let app = Router::new().route(
            "/",
            post(move |headers: HeaderMap, body: Bytes| {
                let captured = captured.clone();
                async move {
                    *captured.lock().await = Some((headers, body.to_vec()));
                    StatusCode::OK
                }
            }),
        );
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let sub = WebhookSubscriber::with_signing_secret(&format!("http://{}", addr), secret);
        let event = Event::RunStatusChanged {
            run_id: Uuid::now_v7(),
            workflow_name: "deploy".to_string(),
            from: RunStatus::Pending,
            to: RunStatus::Running,
            error: None,
            cost_usd: Decimal::ZERO,
            duration_ms: 0,
            labels: HashMap::new(),
            at: Utc::now(),
        };

        sub.handle(&event).await;

        let guard = received.lock().await;
        let (headers, body) = guard.as_ref().expect("request was received");

        let sig_header = headers
            .get("X-Signature-256")
            .expect("X-Signature-256 header must be present")
            .to_str()
            .unwrap();

        assert!(sig_header.starts_with("sha256="));

        // Verify the signature is correct
        let expected = compute_expected_hmac(secret.as_bytes(), body);
        assert_eq!(sig_header, expected);
    }

    #[test]
    fn different_secrets_produce_different_signatures() {
        let body = b"{\"type\":\"run_created\"}";
        let sig_a = WebhookSubscriber::compute_signature("secret-A", body);
        let sig_b = WebhookSubscriber::compute_signature("secret-B", body);
        assert_ne!(sig_a, sig_b);
    }

    #[test]
    fn wrong_secret_does_not_match() {
        let body = b"{\"type\":\"run_created\"}";
        let sig = WebhookSubscriber::compute_signature("correct-secret", body);
        let wrong = compute_expected_hmac(b"wrong-secret", body);
        assert_ne!(sig, wrong);
    }
}
