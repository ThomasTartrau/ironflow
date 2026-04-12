//! [`BetterStackSubscriber`] -- forwards error events to BetterStack Logs.

use std::time::Duration;

use serde::Serialize;
use tokio::time::sleep;
use tracing::{error, info, warn};

use super::{Event, EventSubscriber, SubscriberFuture};

/// Default timeout for outbound HTTP calls.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

/// Maximum number of retry attempts for failed deliveries.
const MAX_RETRIES: u32 = 3;

/// Base delay for exponential backoff (doubled each retry).
const BASE_BACKOFF: Duration = Duration::from_millis(500);

/// Default BetterStack Logs ingestion endpoint.
const DEFAULT_INGEST_URL: &str = "https://in.logs.betterstack.com";

/// Payload sent to BetterStack Logs API.
#[derive(Debug, Serialize)]
struct LogPayload {
    /// ISO-8601 timestamp.
    dt: String,
    /// Log level (always "error" for this subscriber).
    level: &'static str,
    /// Human-readable message.
    message: String,
    /// Structured event data.
    event: serde_json::Value,
}

/// Subscriber that forwards error events to [BetterStack Logs](https://betterstack.com/docs/logs/).
///
/// Only acts on events that represent failures:
/// - [`Event::StepFailed`]
/// - [`Event::RunFailed`]
///
/// All other events are silently ignored (filtering by event type
/// should already be done at subscription time, but this subscriber
/// adds an extra safety check).
///
/// Retries failed deliveries with exponential backoff (up to 3 attempts,
/// 5 s timeout per attempt).
///
/// # Examples
///
/// ```no_run
/// use ironflow_engine::notify::{BetterStackSubscriber, Event, EventPublisher};
///
/// let mut publisher = EventPublisher::new();
/// publisher.subscribe(
///     BetterStackSubscriber::new("my-source-token"),
///     &[Event::STEP_FAILED, Event::RUN_FAILED],
/// );
/// ```
pub struct BetterStackSubscriber {
    source_token: String,
    authorization_header: String,
    ingest_url: String,
    client: reqwest::Client,
}

impl BetterStackSubscriber {
    /// Create a new subscriber with the given BetterStack source token.
    ///
    /// Uses the default ingestion endpoint (`https://in.logs.betterstack.com`).
    ///
    /// # Panics
    ///
    /// Panics if the HTTP client cannot be built (TLS backend unavailable).
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::notify::BetterStackSubscriber;
    ///
    /// let subscriber = BetterStackSubscriber::new("my-source-token");
    /// assert_eq!(subscriber.source_token(), "my-source-token");
    /// ```
    pub fn new(source_token: &str) -> Self {
        Self::with_url(source_token, DEFAULT_INGEST_URL)
    }

    /// Create a subscriber with a custom ingestion URL.
    ///
    /// Useful for testing or self-hosted BetterStack instances.
    ///
    /// # Panics
    ///
    /// Panics if the HTTP client cannot be built (TLS backend unavailable).
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::notify::BetterStackSubscriber;
    ///
    /// let subscriber = BetterStackSubscriber::with_url(
    ///     "my-source-token",
    ///     "https://custom.logs.example.com",
    /// );
    /// assert_eq!(subscriber.ingest_url(), "https://custom.logs.example.com");
    /// ```
    pub fn with_url(source_token: &str, ingest_url: &str) -> Self {
        let client = reqwest::Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .build()
            .expect("failed to build HTTP client");
        Self {
            authorization_header: format!("Bearer {}", source_token),
            source_token: source_token.to_string(),
            ingest_url: ingest_url.to_string(),
            client,
        }
    }

    /// Returns the source token.
    pub fn source_token(&self) -> &str {
        &self.source_token
    }

    /// Returns the ingestion URL.
    pub fn ingest_url(&self) -> &str {
        &self.ingest_url
    }

    /// Build a log payload from an error event. Returns `None` for non-error events.
    fn build_payload(event: &Event) -> Option<LogPayload> {
        match event {
            Event::StepFailed {
                run_id,
                step_id,
                step_name,
                kind,
                error,
                at,
            } => {
                let message = format!(
                    "Step '{}' ({}) failed on run {}: {}",
                    step_name, kind, run_id, error
                );
                let event_json = serde_json::json!({
                    "type": "step_failed",
                    "run_id": run_id.to_string(),
                    "step_id": step_id.to_string(),
                    "step_name": step_name,
                    "kind": kind.to_string(),
                    "error": error,
                });
                Some(LogPayload {
                    dt: at.to_rfc3339(),
                    level: "error",
                    message,
                    event: event_json,
                })
            }
            Event::RunFailed {
                run_id,
                workflow_name,
                error,
                cost_usd,
                duration_ms,
                at,
            } => {
                let error_detail = error.as_deref().unwrap_or("unknown error");
                let message = format!(
                    "Run {} (workflow '{}') failed: {}",
                    run_id, workflow_name, error_detail
                );
                let event_json = serde_json::json!({
                    "type": "run_failed",
                    "run_id": run_id.to_string(),
                    "workflow_name": workflow_name,
                    "error": error_detail,
                    "cost_usd": cost_usd.to_string(),
                    "duration_ms": duration_ms,
                });
                Some(LogPayload {
                    dt: at.to_rfc3339(),
                    level: "error",
                    message,
                    event: event_json,
                })
            }
            _ => None,
        }
    }

    /// Deliver with retry + exponential backoff.
    async fn deliver(&self, payload: &LogPayload) {
        for attempt in 0..MAX_RETRIES {
            let result = self
                .client
                .post(&self.ingest_url)
                .header("Authorization", &self.authorization_header)
                .json(payload)
                .send()
                .await;

            match result {
                Ok(resp) if resp.status().as_u16() == 202 => {
                    info!(
                        ingest_url = %self.ingest_url,
                        message = %payload.message,
                        "betterstack log delivered"
                    );
                    return;
                }
                Ok(resp) => {
                    let status = resp.status();
                    self.log_retry_or_fail(attempt, &payload.message, &format!("HTTP {status}"));
                }
                Err(err) => {
                    self.log_retry_or_fail(attempt, &payload.message, &err.to_string());
                }
            }

            if attempt + 1 < MAX_RETRIES {
                let delay = BASE_BACKOFF * 2u32.pow(attempt);
                sleep(delay).await;
            }
        }
    }

    fn log_retry_or_fail(&self, attempt: u32, message: &str, err_msg: &str) {
        let remaining = MAX_RETRIES - attempt - 1;
        if remaining > 0 {
            warn!(
                ingest_url = %self.ingest_url,
                message,
                attempt = attempt + 1,
                remaining,
                error = %err_msg,
                "betterstack delivery failed, retrying"
            );
        } else {
            error!(
                ingest_url = %self.ingest_url,
                message,
                error = %err_msg,
                "betterstack delivery failed after all retries"
            );
        }
    }
}

impl EventSubscriber for BetterStackSubscriber {
    fn name(&self) -> &str {
        "betterstack"
    }

    fn handle<'a>(&'a self, event: &'a Event) -> SubscriberFuture<'a> {
        Box::pin(async move {
            if let Some(payload) = Self::build_payload(event) {
                self.deliver(&payload).await;
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use ironflow_store::models::{RunStatus, StepKind};
    use rust_decimal::Decimal;
    use uuid::Uuid;

    #[test]
    fn new_sets_default_ingest_url() {
        let sub = BetterStackSubscriber::new("token-123");
        assert_eq!(sub.source_token(), "token-123");
        assert_eq!(sub.ingest_url(), DEFAULT_INGEST_URL);
    }

    #[test]
    fn with_url_sets_custom_ingest_url() {
        let sub = BetterStackSubscriber::with_url("token-123", "https://custom.example.com");
        assert_eq!(sub.source_token(), "token-123");
        assert_eq!(sub.ingest_url(), "https://custom.example.com");
    }

    #[test]
    fn name_is_betterstack() {
        let sub = BetterStackSubscriber::new("token");
        assert_eq!(sub.name(), "betterstack");
    }

    #[test]
    fn build_payload_step_failed() {
        let event = Event::StepFailed {
            run_id: Uuid::now_v7(),
            step_id: Uuid::now_v7(),
            step_name: "build".to_string(),
            kind: StepKind::Shell,
            error: "exit code 1".to_string(),
            at: Utc::now(),
        };

        let payload = BetterStackSubscriber::build_payload(&event);
        assert!(payload.is_some());
        let payload = payload.unwrap();
        assert_eq!(payload.level, "error");
        assert!(payload.message.contains("build"));
        assert!(payload.message.contains("exit code 1"));
        assert_eq!(payload.event["type"], "step_failed");
        assert_eq!(payload.event["error"], "exit code 1");
    }

    #[test]
    fn build_payload_run_failed() {
        let event = Event::RunFailed {
            run_id: Uuid::now_v7(),
            workflow_name: "deploy".to_string(),
            error: Some("step 'build' failed".to_string()),
            cost_usd: Decimal::new(42, 2),
            duration_ms: 5000,
            at: Utc::now(),
        };

        let payload = BetterStackSubscriber::build_payload(&event);
        assert!(payload.is_some());
        let payload = payload.unwrap();
        assert_eq!(payload.level, "error");
        assert!(payload.message.contains("deploy"));
        assert!(payload.message.contains("step 'build' failed"));
        assert_eq!(payload.event["type"], "run_failed");
        assert_eq!(payload.event["workflow_name"], "deploy");
    }

    #[test]
    fn build_payload_run_failed_without_error_message() {
        let event = Event::RunFailed {
            run_id: Uuid::now_v7(),
            workflow_name: "deploy".to_string(),
            error: None,
            cost_usd: Decimal::ZERO,
            duration_ms: 1000,
            at: Utc::now(),
        };

        let payload = BetterStackSubscriber::build_payload(&event).unwrap();
        assert!(payload.message.contains("unknown error"));
        assert_eq!(payload.event["error"], "unknown error");
    }

    #[test]
    fn build_payload_run_completed_returns_none() {
        let event = Event::RunStatusChanged {
            run_id: Uuid::now_v7(),
            workflow_name: "deploy".to_string(),
            from: RunStatus::Running,
            to: RunStatus::Completed,
            error: None,
            cost_usd: Decimal::ZERO,
            duration_ms: 1000,
            at: Utc::now(),
        };

        assert!(BetterStackSubscriber::build_payload(&event).is_none());
    }

    #[test]
    fn build_payload_run_created_returns_none() {
        let event = Event::RunCreated {
            run_id: Uuid::now_v7(),
            workflow_name: "deploy".to_string(),
            at: Utc::now(),
        };

        assert!(BetterStackSubscriber::build_payload(&event).is_none());
    }

    #[test]
    fn build_payload_step_completed_returns_none() {
        let event = Event::StepCompleted {
            run_id: Uuid::now_v7(),
            step_id: Uuid::now_v7(),
            step_name: "build".to_string(),
            kind: StepKind::Shell,
            duration_ms: 500,
            cost_usd: Decimal::ZERO,
            at: Utc::now(),
        };

        assert!(BetterStackSubscriber::build_payload(&event).is_none());
    }

    #[test]
    fn build_payload_approval_requested_returns_none() {
        let event = Event::ApprovalRequested {
            run_id: Uuid::now_v7(),
            step_id: Uuid::now_v7(),
            message: "Deploy to prod?".to_string(),
            at: Utc::now(),
        };

        assert!(BetterStackSubscriber::build_payload(&event).is_none());
    }

    #[test]
    fn build_payload_user_signed_in_returns_none() {
        let event = Event::UserSignedIn {
            user_id: Uuid::now_v7(),
            username: "alice".to_string(),
            at: Utc::now(),
        };

        assert!(BetterStackSubscriber::build_payload(&event).is_none());
    }

    #[tokio::test]
    async fn handle_ignores_non_error_events() {
        let sub = BetterStackSubscriber::with_url("token", "http://127.0.0.1:1");
        let event = Event::RunCreated {
            run_id: Uuid::now_v7(),
            workflow_name: "deploy".to_string(),
            at: Utc::now(),
        };
        // Should return immediately without attempting HTTP
        sub.handle(&event).await;
    }

    #[tokio::test]
    async fn deliver_to_real_endpoint_returns_202() {
        use axum::Router;
        use axum::http::StatusCode;
        use axum::routing::post;
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let app = Router::new().route("/", post(|| async { StatusCode::ACCEPTED }));

        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let sub = BetterStackSubscriber::with_url("test-token", &format!("http://{}", addr));
        let event = Event::StepFailed {
            run_id: Uuid::now_v7(),
            step_id: Uuid::now_v7(),
            step_name: "build".to_string(),
            kind: StepKind::Shell,
            error: "exit code 1".to_string(),
            at: Utc::now(),
        };

        sub.handle(&event).await;
    }
}
