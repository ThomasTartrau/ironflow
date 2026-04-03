//! [`WebhookSubscriber`] -- POSTs events as JSON to a URL.

use std::time::Duration;

use tracing::{error, info, warn};

use super::{Event, EventSubscriber, SubscriberFuture};

/// Default timeout for outbound HTTP calls.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

/// Maximum number of retry attempts for failed deliveries.
const MAX_RETRIES: u32 = 3;

/// Base delay for exponential backoff (doubled each retry).
const BASE_BACKOFF: Duration = Duration::from_millis(500);

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
    client: reqwest::Client,
}

impl WebhookSubscriber {
    /// Create a new webhook subscriber targeting the given URL.
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
        let client = reqwest::Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .build()
            .expect("failed to build HTTP client");
        Self {
            url: url.to_string(),
            client,
        }
    }

    /// Returns the target URL.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Deliver with retry + exponential backoff.
    async fn deliver(&self, event: &Event) {
        for attempt in 0..MAX_RETRIES {
            let result = self.client.post(&self.url).json(event).send().await;

            match result {
                Ok(resp) if resp.status().is_success() => {
                    info!(
                        url = %self.url,
                        event_type = %event.event_type(),
                        "webhook delivered"
                    );
                    return;
                }
                Ok(resp) => {
                    let status = resp.status();
                    self.log_retry_or_fail(attempt, event.event_type(), &format!("HTTP {status}"));
                }
                Err(err) => {
                    self.log_retry_or_fail(attempt, event.event_type(), &err.to_string());
                }
            }

            if attempt + 1 < MAX_RETRIES {
                let delay = BASE_BACKOFF * 2u32.pow(attempt);
                tokio::time::sleep(delay).await;
            }
        }
    }

    fn log_retry_or_fail(&self, attempt: u32, event_type: &str, err_msg: &str) {
        let remaining = MAX_RETRIES - attempt - 1;
        if remaining > 0 {
            warn!(
                url = %self.url,
                event_type,
                attempt = attempt + 1,
                remaining,
                error = %err_msg,
                "webhook delivery failed, retrying"
            );
        } else {
            error!(
                url = %self.url,
                event_type,
                error = %err_msg,
                "webhook delivery failed after all retries"
            );
        }
    }
}

impl EventSubscriber for WebhookSubscriber {
    fn name(&self) -> &str {
        "webhook"
    }

    fn handle<'a>(&'a self, event: &'a Event) -> SubscriberFuture<'a> {
        Box::pin(async move {
            self.deliver(event).await;
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
