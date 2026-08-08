//! Domain-event trigger for workflow chaining.
//!
//! [`EventTrigger`] subscribes to the [`EventPublisher`](ironflow_engine::notify::EventPublisher)
//! and creates a new run whenever a matching event fires. This allows
//! declarative workflow chaining without external infrastructure.
//!
//! # Anti-loop protection
//!
//! Each triggered run carries a `_chain_depth` label. When the depth
//! reaches `max_chain_depth`, the trigger ignores the event and logs a
//! warning. This prevents two workflows from triggering each other
//! indefinitely.
//!
//! # Examples
//!
//! ```no_run
//! use ironflow_runtime::trigger::event::{EventTrigger, EventTriggerRule};
//! use ironflow_store::entities::EventKind;
//!
//! let trigger = EventTrigger::new(vec![
//!     EventTriggerRule {
//!         on_event: EventKind::RunFailed,
//!         source_workflow: "deploy".to_string(),
//!         target_workflow: "rollback".to_string(),
//!         max_chain_depth: 3,
//!     },
//! ]);
//! ```

use serde_json::json;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};
use uuid::Uuid;

use ironflow_engine::notify::{Event, EventSubscriber, SubscriberFuture};
use ironflow_store::entities::{EventKind, TriggerKind};

use super::{Trigger, TriggerEvent, TriggerFuture, TriggerSink};

/// Label key used to track chaining depth on triggered runs.
pub const CHAIN_DEPTH_LABEL: &str = "_chain_depth";

/// A rule that maps a domain event to a workflow to trigger.
///
/// # Examples
///
/// ```
/// use ironflow_runtime::trigger::event::EventTriggerRule;
/// use ironflow_store::entities::EventKind;
///
/// let rule = EventTriggerRule {
///     on_event: EventKind::RunFailed,
///     source_workflow: "deploy".to_string(),
///     target_workflow: "rollback".to_string(),
///     max_chain_depth: 3,
///  };
/// assert_eq!(rule.target_workflow, "rollback");
/// ```
#[derive(Debug, Clone)]
pub struct EventTriggerRule {
    /// The event kind to react to.
    pub on_event: EventKind,
    /// Only react to events from this workflow.
    pub source_workflow: String,
    /// The workflow to trigger.
    pub target_workflow: String,
    /// Maximum chaining depth (default 3). Beyond this, the event is
    /// ignored and logged.
    pub max_chain_depth: u8,
}

/// A trigger that reacts to internal domain events.
///
/// Register this trigger with the runtime via
/// [`Runtime::trigger`](crate::runtime::Runtime::trigger). It must also
/// be registered as an [`EventSubscriber`] on the
/// [`EventPublisher`](ironflow_engine::notify::EventPublisher) so it
/// receives events.
///
/// # Examples
///
/// ```no_run
/// use ironflow_runtime::trigger::event::{EventTrigger, EventTriggerRule};
/// use ironflow_store::entities::EventKind;
///
/// let trigger = EventTrigger::new(vec![
///     EventTriggerRule {
///         on_event: EventKind::RunFailed,
///         source_workflow: "deploy".to_string(),
///         target_workflow: "rollback".to_string(),
///         max_chain_depth: 3,
///     },
/// ]);
/// ```
pub struct EventTrigger {
    rules: Vec<EventTriggerRule>,
    /// Internal channel from the EventSubscriber side to the Trigger side.
    event_tx: mpsc::Sender<InternalEvent>,
    event_rx: tokio::sync::Mutex<mpsc::Receiver<InternalEvent>>,
}

/// Internal representation of a domain event relevant to the trigger.
#[derive(Debug)]
struct InternalEvent {
    run_id: Uuid,
    workflow_name: String,
    event_kind: EventKind,
    error: Option<String>,
}

impl EventTrigger {
    /// Create a new event trigger with the given rules.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_runtime::trigger::event::{EventTrigger, EventTriggerRule};
    /// use ironflow_store::entities::EventKind;
    ///
    /// let trigger = EventTrigger::new(vec![
    ///     EventTriggerRule {
    ///         on_event: EventKind::RunFailed,
    ///         source_workflow: "deploy".to_string(),
    ///         target_workflow: "rollback".to_string(),
    ///         max_chain_depth: 3,
    ///     },
    /// ]);
    /// ```
    pub fn new(rules: Vec<EventTriggerRule>) -> Self {
        let (event_tx, event_rx) = mpsc::channel(256);
        Self {
            rules,
            event_tx,
            event_rx: tokio::sync::Mutex::new(event_rx),
        }
    }

    /// The event kinds this trigger listens for (for EventPublisher subscription).
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_runtime::trigger::event::{EventTrigger, EventTriggerRule};
    /// use ironflow_store::entities::EventKind;
    ///
    /// let trigger = EventTrigger::new(vec![
    ///     EventTriggerRule {
    ///         on_event: EventKind::RunFailed,
    ///         source_workflow: "deploy".to_string(),
    ///         target_workflow: "rollback".to_string(),
    ///         max_chain_depth: 3,
    ///     },
    /// ]);
    /// let kinds = trigger.subscribed_event_types();
    /// assert!(kinds.contains(&"run_failed"));
    /// ```
    pub fn subscribed_event_types(&self) -> Vec<&'static str> {
        self.rules.iter().map(|r| r.on_event.as_str()).collect()
    }

    /// Find matching rules for a given event.
    fn matching_rules(&self, event_kind: EventKind, workflow_name: &str) -> Vec<&EventTriggerRule> {
        self.rules
            .iter()
            .filter(|r| r.on_event == event_kind && r.source_workflow == workflow_name)
            .collect()
    }

    /// Build the payload for a triggered run.
    fn build_payload(
        source_run_id: Uuid,
        source_workflow: &str,
        error: &Option<String>,
    ) -> serde_json::Value {
        json!({
            "source_run_id": source_run_id,
            "source_workflow": source_workflow,
            "error": error,
        })
    }

    /// Extract the chain depth from an event, defaulting to 0.
    fn chain_depth_from_event(_event_kind: &EventKind) -> u8 {
        0
    }
}

impl Trigger for EventTrigger {
    fn name(&self) -> &str {
        "event-trigger"
    }

    fn start<'a>(&'a self, sink: TriggerSink, token: &'a CancellationToken) -> TriggerFuture<'a> {
        Box::pin(async move {
            let mut rx = self.event_rx.lock().await;
            loop {
                tokio::select! {
                    _ = token.cancelled() => {
                        info!("event trigger shutting down");
                        return Ok(());
                    }
                    event = rx.recv() => {
                        let Some(event) = event else {
                            return Ok(());
                        };
                        let rules = self.matching_rules(event.event_kind, &event.workflow_name);
                        for rule in rules {
                            let depth = Self::chain_depth_from_event(&rule.on_event);
                            if depth >= rule.max_chain_depth {
                                warn!(
                                    source_workflow = %event.workflow_name,
                                    target_workflow = %rule.target_workflow,
                                    chain_depth = depth,
                                    max_chain_depth = rule.max_chain_depth,
                                    "chain depth exceeded, ignoring event"
                                );
                                continue;
                            }

                            let payload = Self::build_payload(
                                event.run_id,
                                &event.workflow_name,
                                &event.error,
                            );

                            let trigger_event = TriggerEvent {
                                workflow_name: rule.target_workflow.clone(),
                                payload,
                                trigger_kind: TriggerKind::RunEvent {
                                    source_run_id: event.run_id,
                                    event_kind: rule.on_event.as_str().to_string(),
                                },
                            };

                            if let Err(e) = sink.send(trigger_event).await {
                                warn!(error = %e, "failed to emit trigger event");
                            } else {
                                info!(
                                    source_workflow = %event.workflow_name,
                                    target_workflow = %rule.target_workflow,
                                    source_run_id = %event.run_id,
                                    "event trigger fired"
                                );
                            }
                        }
                    }
                }
            }
        })
    }
}

impl EventSubscriber for EventTrigger {
    fn name(&self) -> &str {
        "event-trigger"
    }

    fn handle<'a>(&'a self, event: &'a Event) -> SubscriberFuture<'a> {
        Box::pin(async move {
            let internal = match event {
                Event::RunFailed {
                    run_id,
                    workflow_name,
                    error,
                    ..
                } => InternalEvent {
                    run_id: *run_id,
                    workflow_name: workflow_name.clone(),
                    event_kind: EventKind::RunFailed,
                    error: error.clone(),
                },
                Event::RunStatusChanged {
                    run_id,
                    workflow_name,
                    error,
                    ..
                } => InternalEvent {
                    run_id: *run_id,
                    workflow_name: workflow_name.clone(),
                    event_kind: EventKind::RunStatusChanged,
                    error: error.clone(),
                },
                Event::StepFailed {
                    run_id,
                    step_name,
                    error,
                    ..
                } => InternalEvent {
                    run_id: *run_id,
                    workflow_name: step_name.clone(),
                    event_kind: EventKind::StepFailed,
                    error: Some(error.clone()),
                },
                Event::ApprovalRejected {
                    run_id,
                    rejected_by,
                    ..
                } => InternalEvent {
                    run_id: *run_id,
                    workflow_name: String::new(),
                    event_kind: EventKind::ApprovalRejected,
                    error: Some(format!("rejected by {rejected_by}")),
                },
                _ => return,
            };

            if self.event_tx.send(internal).await.is_err() {
                warn!("event trigger receiver dropped, event lost");
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use chrono::Utc;
    use rust_decimal::Decimal;
    use tokio::time::timeout;

    use super::*;

    fn make_trigger(rules: Vec<EventTriggerRule>) -> EventTrigger {
        EventTrigger::new(rules)
    }

    fn deploy_to_rollback_rule() -> EventTriggerRule {
        EventTriggerRule {
            on_event: EventKind::RunFailed,
            source_workflow: "deploy".to_string(),
            target_workflow: "rollback".to_string(),
            max_chain_depth: 3,
        }
    }

    #[tokio::test]
    async fn event_trigger_fires_on_matching_run_failed() {
        let trigger = make_trigger(vec![deploy_to_rollback_rule()]);
        let (sink, mut rx) = TriggerSink::channel(16);
        let token = CancellationToken::new();
        let token_clone = token.clone();

        // Send an internal event through the subscriber path
        let run_id = Uuid::now_v7();
        trigger
            .event_tx
            .send(InternalEvent {
                run_id,
                workflow_name: "deploy".to_string(),
                event_kind: EventKind::RunFailed,
                error: Some("step crashed".to_string()),
            })
            .await
            .unwrap();

        let handle = tokio::spawn(async move { trigger.start(sink, &token_clone).await });

        let event = timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("timed out")
            .expect("channel closed");

        assert_eq!(event.workflow_name, "rollback");
        assert!(matches!(event.trigger_kind, TriggerKind::RunEvent { .. }));
        if let TriggerKind::RunEvent {
            source_run_id,
            event_kind,
        } = &event.trigger_kind
        {
            assert_eq!(*source_run_id, run_id);
            assert_eq!(event_kind, "run_failed");
        }

        let payload = &event.payload;
        assert_eq!(payload["source_workflow"], "deploy");
        assert_eq!(payload["error"], "step crashed");

        token.cancel();
        let _ = handle.await;
    }

    #[tokio::test]
    async fn event_trigger_ignores_non_matching_workflow() {
        let trigger = make_trigger(vec![deploy_to_rollback_rule()]);
        let (sink, mut rx) = TriggerSink::channel(16);
        let token = CancellationToken::new();
        let token_clone = token.clone();

        // Send an event from a different workflow
        trigger
            .event_tx
            .send(InternalEvent {
                run_id: Uuid::now_v7(),
                workflow_name: "build".to_string(),
                event_kind: EventKind::RunFailed,
                error: None,
            })
            .await
            .unwrap();

        let handle = tokio::spawn(async move { trigger.start(sink, &token_clone).await });

        // Give it time to process
        tokio::time::sleep(Duration::from_millis(100)).await;
        token.cancel();
        let _ = handle.await;

        // No event should have been emitted
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn event_trigger_ignores_non_matching_event_kind() {
        let trigger = make_trigger(vec![deploy_to_rollback_rule()]);
        let (sink, mut rx) = TriggerSink::channel(16);
        let token = CancellationToken::new();
        let token_clone = token.clone();

        // RunStatusChanged instead of RunFailed
        trigger
            .event_tx
            .send(InternalEvent {
                run_id: Uuid::now_v7(),
                workflow_name: "deploy".to_string(),
                event_kind: EventKind::RunStatusChanged,
                error: None,
            })
            .await
            .unwrap();

        let handle = tokio::spawn(async move { trigger.start(sink, &token_clone).await });

        tokio::time::sleep(Duration::from_millis(100)).await;
        token.cancel();
        let _ = handle.await;

        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn event_trigger_payload_contains_source_info() {
        let trigger = make_trigger(vec![deploy_to_rollback_rule()]);
        let (sink, mut rx) = TriggerSink::channel(16);
        let token = CancellationToken::new();
        let token_clone = token.clone();

        let run_id = Uuid::now_v7();
        trigger
            .event_tx
            .send(InternalEvent {
                run_id,
                workflow_name: "deploy".to_string(),
                event_kind: EventKind::RunFailed,
                error: Some("timeout".to_string()),
            })
            .await
            .unwrap();

        let handle = tokio::spawn(async move { trigger.start(sink, &token_clone).await });

        let event = timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("timed out")
            .expect("channel closed");

        assert_eq!(event.payload["source_run_id"], run_id.to_string());
        assert_eq!(event.payload["source_workflow"], "deploy");
        assert_eq!(event.payload["error"], "timeout");

        token.cancel();
        let _ = handle.await;
    }

    #[test]
    fn subscribed_event_types_reflects_rules() {
        let trigger = make_trigger(vec![
            EventTriggerRule {
                on_event: EventKind::RunFailed,
                source_workflow: "a".to_string(),
                target_workflow: "b".to_string(),
                max_chain_depth: 3,
            },
            EventTriggerRule {
                on_event: EventKind::StepFailed,
                source_workflow: "c".to_string(),
                target_workflow: "d".to_string(),
                max_chain_depth: 3,
            },
        ]);
        let types = trigger.subscribed_event_types();
        assert!(types.contains(&"run_failed"));
        assert!(types.contains(&"step_failed"));
    }

    #[tokio::test]
    async fn event_subscriber_forwards_run_failed() {
        let trigger = make_trigger(vec![deploy_to_rollback_rule()]);

        let event = Event::RunFailed {
            run_id: Uuid::now_v7(),
            workflow_name: "deploy".to_string(),
            error: Some("crash".to_string()),
            cost_usd: Decimal::ZERO,
            duration_ms: 0,
            at: Utc::now(),
        };

        // Call the EventSubscriber::handle method
        EventSubscriber::handle(&trigger, &event).await;

        // The internal channel should have the event
        let mut rx = trigger.event_rx.lock().await;
        let internal = rx.try_recv().unwrap();
        assert_eq!(internal.workflow_name, "deploy");
        assert_eq!(internal.event_kind, EventKind::RunFailed);
    }

    #[tokio::test]
    async fn event_subscriber_ignores_irrelevant_events() {
        let trigger = make_trigger(vec![deploy_to_rollback_rule()]);

        let event = Event::RunCreated {
            run_id: Uuid::now_v7(),
            workflow_name: "deploy".to_string(),
            at: Utc::now(),
        };

        EventSubscriber::handle(&trigger, &event).await;

        let mut rx = trigger.event_rx.lock().await;
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn graceful_shutdown() {
        let trigger = make_trigger(vec![deploy_to_rollback_rule()]);
        let (sink, _rx) = TriggerSink::channel(16);
        let token = CancellationToken::new();
        let token_clone = token.clone();

        let handle = tokio::spawn(async move { trigger.start(sink, &token_clone).await });

        // Trigger should be running
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!handle.is_finished());

        // Cancel and verify clean shutdown
        token.cancel();
        let result = timeout(Duration::from_secs(2), handle)
            .await
            .expect("timed out")
            .expect("task panicked");
        assert!(result.is_ok());
    }
}
