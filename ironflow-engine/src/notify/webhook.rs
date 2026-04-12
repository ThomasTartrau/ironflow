//! [`WebhookSubscriber`] -- POSTs events as JSON to a URL.

use reqwest::Client;

use super::retry::{RetryConfig, deliver_with_retry, is_success_2xx};
use super::{Event, EventSubscriber, SubscriberFuture};

/// Subscriber that POSTs the event as JSON to a webhook URL.
///
/// Retries failed deliveries with exponential backoff (up to 3 attempts,
/// 5 s timeout per attempt). The HTTP client is created once and reused.
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
/// publisher.subscribe(
///     WebhookSubscriber::new("https://hooks.example.com/events"),
///     &[Event::RUN_STATUS_CHANGED, Event::STEP_FAILED],
/// );
/// ```
pub struct WebhookSubscriber {
    url: String,
    client: Client,
    retry_config: RetryConfig,
}

impl WebhookSubscriber {
    /// Create a new webhook subscriber targeting the given URL.
    ///
    /// Uses the default [`RetryConfig`] (3 retries, 5 s timeout, 500 ms
    /// base backoff).
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
    /// ```
    pub fn new(url: &str) -> Self {
        Self::with_retry_config(url, RetryConfig::default())
    }

    /// Create a webhook subscriber with a custom retry configuration.
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
        let client = retry_config.build_client();
        Self {
            url: url.to_string(),
            client,
            retry_config,
        }
    }

    /// Returns the target URL.
    pub fn url(&self) -> &str {
        &self.url
    }
}

impl EventSubscriber for WebhookSubscriber {
    fn name(&self) -> &str {
        "webhook"
    }

    fn handle<'a>(&'a self, event: &'a Event) -> SubscriberFuture<'a> {
        Box::pin(async move {
            deliver_with_retry(
                &self.retry_config,
                || self.client.post(&self.url).json(event),
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
    use super::*;

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
}
