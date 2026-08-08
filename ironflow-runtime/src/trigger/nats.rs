//! NATS JetStream trigger for consuming messages from a subject.
//!
//! [`NatsTrigger`] connects to a NATS server and consumes messages from
//! configured subjects. Each message creates a workflow run whose payload
//! is the message body.
//!
//! # Feature flag
//!
//! This module is only available when the `trigger-nats` feature is enabled.
//!
//! # Reconnection
//!
//! The [`async_nats`] client handles reconnection automatically. The
//! trigger logs disconnections and reconnections but does not stop.
//!
//! # Dead letter
//!
//! Messages that fail processing are nak'd with a delay. After
//! [`NatsSubjectMapping::max_deliveries`] attempts, the message is
//! terminated (sent to the dead-letter queue configured on the NATS
//! stream).
//!
//! # Examples
//!
//! ```no_run
//! use ironflow_runtime::trigger::nats::{NatsTrigger, NatsTriggerConfig, NatsSubjectMapping};
//!
//! let trigger = NatsTrigger::new(NatsTriggerConfig {
//!     url: "nats://localhost:4222".to_string(),
//!     subjects: vec![
//!         NatsSubjectMapping {
//!             subject: "workflows.deploy".to_string(),
//!             queue_group: "ironflow".to_string(),
//!             workflow_name: "deploy".to_string(),
//!             max_deliveries: 3,
//!         },
//!     ],
//! });
//! ```

use std::time::Duration;

use async_nats::jetstream;
use async_nats::jetstream::consumer::PullConsumer;
use futures_util::StreamExt;
use serde_json::Value;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use ironflow_store::entities::TriggerKind;

use super::{Trigger, TriggerError, TriggerEvent, TriggerFuture, TriggerSink};

/// Configuration for a single NATS subject mapping.
///
/// # Examples
///
/// ```
/// use ironflow_runtime::trigger::nats::NatsSubjectMapping;
///
/// let mapping = NatsSubjectMapping {
///     subject: "orders.created".to_string(),
///     queue_group: "ironflow".to_string(),
///     workflow_name: "process-order".to_string(),
///     max_deliveries: 3,
/// };
/// assert_eq!(mapping.workflow_name, "process-order");
/// ```
#[derive(Debug, Clone)]
pub struct NatsSubjectMapping {
    /// The NATS subject to subscribe to.
    pub subject: String,
    /// Queue group name for load balancing across instances.
    pub queue_group: String,
    /// The workflow to trigger for messages on this subject.
    pub workflow_name: String,
    /// Maximum delivery attempts before terminating the message.
    pub max_deliveries: u32,
}

/// Configuration for the NATS trigger.
///
/// # Examples
///
/// ```
/// use ironflow_runtime::trigger::nats::{NatsTriggerConfig, NatsSubjectMapping};
///
/// let config = NatsTriggerConfig {
///     url: "nats://localhost:4222".to_string(),
///     subjects: vec![
///         NatsSubjectMapping {
///             subject: "workflows.>".to_string(),
///             queue_group: "ironflow".to_string(),
///             workflow_name: "deploy".to_string(),
///             max_deliveries: 3,
///         },
///     ],
/// };
/// ```
#[derive(Debug, Clone)]
pub struct NatsTriggerConfig {
    /// NATS server URL.
    pub url: String,
    /// Subject-to-workflow mappings.
    pub subjects: Vec<NatsSubjectMapping>,
}

/// A trigger that consumes messages from NATS JetStream.
///
/// # Examples
///
/// ```no_run
/// use ironflow_runtime::trigger::nats::{NatsTrigger, NatsTriggerConfig, NatsSubjectMapping};
///
/// let trigger = NatsTrigger::new(NatsTriggerConfig {
///     url: "nats://localhost:4222".to_string(),
///     subjects: vec![
///         NatsSubjectMapping {
///             subject: "workflows.deploy".to_string(),
///             queue_group: "ironflow".to_string(),
///             workflow_name: "deploy".to_string(),
///             max_deliveries: 3,
///         },
///     ],
/// });
/// ```
pub struct NatsTrigger {
    config: NatsTriggerConfig,
}

impl NatsTrigger {
    /// Create a new NATS trigger with the given configuration.
    pub fn new(config: NatsTriggerConfig) -> Self {
        Self { config }
    }

    /// Process a single message: parse, send to sink, ack/nak.
    async fn process_message(
        message: jetstream::Message,
        mapping: &NatsSubjectMapping,
        sink: &TriggerSink,
    ) {
        let delivery_count = message.info().map(|info| info.delivered).unwrap_or(1);

        let payload: Value = match serde_json::from_slice(&message.payload) {
            Ok(v) => v,
            Err(e) => {
                warn!(
                    subject = %mapping.subject,
                    error = %e,
                    delivery = delivery_count,
                    "invalid JSON in NATS message"
                );
                if delivery_count >= i64::from(mapping.max_deliveries) {
                    info!(
                        subject = %mapping.subject,
                        "max deliveries reached, terminating message"
                    );
                    if let Err(e) = message.ack_with(async_nats::jetstream::AckKind::Term).await {
                        error!(error = %e, "failed to terminate message");
                    }
                } else {
                    if let Err(e) = message
                        .ack_with(async_nats::jetstream::AckKind::Nak(Some(
                            Duration::from_secs(5 * delivery_count.unsigned_abs()),
                        )))
                        .await
                    {
                        error!(error = %e, "failed to nak message");
                    }
                }
                return;
            }
        };

        let event = TriggerEvent {
            workflow_name: mapping.workflow_name.clone(),
            payload,
            trigger_kind: TriggerKind::Nats {
                subject: mapping.subject.clone(),
            },
        };

        match sink.send(event).await {
            Ok(()) => {
                if let Err(e) = message.ack().await {
                    error!(error = %e, "failed to ack NATS message");
                }
                info!(
                    subject = %mapping.subject,
                    workflow = %mapping.workflow_name,
                    "NATS message processed, run creation requested"
                );
            }
            Err(e) => {
                warn!(error = %e, "failed to send trigger event, nak'ing message");
                if let Err(e) = message
                    .ack_with(async_nats::jetstream::AckKind::Nak(Some(
                        Duration::from_secs(5),
                    )))
                    .await
                {
                    error!(error = %e, "failed to nak message");
                }
            }
        }
    }

    /// Consume messages from a single subject mapping.
    async fn consume_subject(
        consumer: PullConsumer,
        mapping: NatsSubjectMapping,
        sink: TriggerSink,
        token: CancellationToken,
    ) {
        let mut messages = match consumer.messages().await {
            Ok(m) => m,
            Err(e) => {
                error!(
                    subject = %mapping.subject,
                    error = %e,
                    "failed to start consuming messages"
                );
                return;
            }
        };

        loop {
            tokio::select! {
                _ = token.cancelled() => {
                    info!(subject = %mapping.subject, "NATS consumer shutting down");
                    return;
                }
                msg = messages.next() => {
                    match msg {
                        Some(Ok(message)) => {
                            Self::process_message(message, &mapping, &sink).await;
                        }
                        Some(Err(e)) => {
                            warn!(
                                subject = %mapping.subject,
                                error = %e,
                                "error receiving NATS message"
                            );
                        }
                        None => {
                            info!(subject = %mapping.subject, "NATS message stream ended");
                            return;
                        }
                    }
                }
            }
        }
    }
}

impl Trigger for NatsTrigger {
    fn name(&self) -> &str {
        "nats-trigger"
    }

    fn start<'a>(&'a self, sink: TriggerSink, token: &'a CancellationToken) -> TriggerFuture<'a> {
        Box::pin(async move {
            let client = async_nats::connect(&self.config.url)
                .await
                .map_err(|e| TriggerError::Failed(format!("NATS connection failed: {e}")))?;

            info!(url = %self.config.url, "connected to NATS");

            let jetstream = jetstream::new(client);

            let mut handles = Vec::new();

            for mapping in &self.config.subjects {
                let consumer_name = format!("ironflow-{}", mapping.workflow_name);

                let stream = jetstream.get_stream(&mapping.subject).await.map_err(|e| {
                    TriggerError::Failed(format!(
                        "failed to get stream for {}: {e}",
                        mapping.subject
                    ))
                })?;

                let consumer = stream
                    .get_or_create_consumer(
                        &consumer_name,
                        jetstream::consumer::pull::Config {
                            durable_name: Some(consumer_name.clone()),
                            filter_subject: mapping.subject.clone(),
                            max_deliver: mapping.max_deliveries as i64,
                            ..Default::default()
                        },
                    )
                    .await
                    .map_err(|e| {
                        TriggerError::Failed(format!(
                            "failed to create consumer for {}: {e}",
                            mapping.subject
                        ))
                    })?;

                info!(
                    subject = %mapping.subject,
                    workflow = %mapping.workflow_name,
                    consumer = %consumer_name,
                    "NATS consumer started"
                );

                let handle = tokio::spawn(Self::consume_subject(
                    consumer,
                    mapping.clone(),
                    sink.clone(),
                    token.clone(),
                ));
                handles.push(handle);
            }

            token.cancelled().await;

            for handle in handles {
                let _ = handle.await;
            }

            info!("NATS trigger shut down");
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_creation() {
        let config = NatsTriggerConfig {
            url: "nats://localhost:4222".to_string(),
            subjects: vec![NatsSubjectMapping {
                subject: "orders.created".to_string(),
                queue_group: "ironflow".to_string(),
                workflow_name: "process-order".to_string(),
                max_deliveries: 3,
            }],
        };
        assert_eq!(config.subjects.len(), 1);
        assert_eq!(config.subjects[0].workflow_name, "process-order");
    }

    #[test]
    fn trigger_name() {
        let trigger = NatsTrigger::new(NatsTriggerConfig {
            url: "nats://localhost:4222".to_string(),
            subjects: vec![],
        });
        assert_eq!(trigger.name(), "nats-trigger");
    }

    #[tokio::test]
    #[ignore = "requires a running NATS server on localhost:4222"]
    async fn nats_trigger_connects_and_shuts_down() {
        let trigger = NatsTrigger::new(NatsTriggerConfig {
            url: "nats://localhost:4222".to_string(),
            subjects: vec![],
        });

        let (sink, _rx) = TriggerSink::channel(16);
        let token = CancellationToken::new();
        let token_clone = token.clone();

        let handle = tokio::spawn(async move { trigger.start(sink, &token_clone).await });

        tokio::time::sleep(Duration::from_millis(100)).await;
        token.cancel();

        let result = tokio::time::timeout(Duration::from_secs(5), handle)
            .await
            .expect("timed out")
            .expect("task panicked");
        assert!(result.is_ok());
    }

    #[tokio::test]
    #[ignore = "requires a running NATS server on localhost:4222 with JetStream"]
    async fn nats_trigger_processes_message() {
        use async_nats::jetstream;

        let nats_url = "nats://localhost:4222";
        let subject = "test-ironflow-trigger";
        let stream_name = "test-ironflow-stream";

        let client = async_nats::connect(nats_url).await.unwrap();
        let js = jetstream::new(client.clone());

        // Create a test stream
        js.get_or_create_stream(jetstream::stream::Config {
            name: stream_name.to_string(),
            subjects: vec![subject.to_string()],
            ..Default::default()
        })
        .await
        .unwrap();

        let trigger = NatsTrigger::new(NatsTriggerConfig {
            url: nats_url.to_string(),
            subjects: vec![NatsSubjectMapping {
                subject: subject.to_string(),
                queue_group: "test-ironflow".to_string(),
                workflow_name: "test-workflow".to_string(),
                max_deliveries: 3,
            }],
        });

        let (sink, mut rx) = TriggerSink::channel(16);
        let token = CancellationToken::new();
        let token_clone = token.clone();

        let handle = tokio::spawn(async move { trigger.start(sink, &token_clone).await });

        // Publish a message
        tokio::time::sleep(Duration::from_millis(500)).await;
        js.publish(
            subject,
            serde_json::json!({"key": "value"}).to_string().into(),
        )
        .await
        .unwrap()
        .await
        .unwrap();

        // Receive the trigger event
        let event = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("timed out")
            .expect("channel closed");

        assert_eq!(event.workflow_name, "test-workflow");
        assert_eq!(event.payload["key"], "value");
        assert!(matches!(event.trigger_kind, TriggerKind::Nats { .. }));

        token.cancel();
        let _ = handle.await;

        // Cleanup
        js.delete_stream(stream_name).await.unwrap();
    }
}
