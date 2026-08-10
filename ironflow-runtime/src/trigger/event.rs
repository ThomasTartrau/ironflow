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
//!         conditions: vec![],
//!     },
//! ]);
//! ```

use std::collections::HashMap;
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use rust_decimal::Decimal;
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

/// Context available to [`TriggerCondition`]s when evaluating whether a
/// rule should fire.
///
/// Exposes metadata from the source run: labels, error message,
/// aggregated cost and duration.
///
/// # Examples
///
/// ```
/// use std::collections::HashMap;
/// use ironflow_runtime::trigger::event::TriggerContext;
/// use rust_decimal::Decimal;
///
/// let ctx = TriggerContext {
///     labels: HashMap::from([("env".to_string(), "prod".to_string())]),
///     error: Some("timeout".to_string()),
///     cost_usd: Decimal::new(42, 2),
///     duration_ms: 5000,
/// };
/// assert_eq!(ctx.labels.get("env").unwrap(), "prod");
/// ```
#[derive(Debug, Clone)]
pub struct TriggerContext {
    /// Labels of the source run.
    pub labels: HashMap<String, String>,
    /// Error message of the source run (if any).
    pub error: Option<String>,
    /// Aggregated cost in USD of the source run.
    pub cost_usd: Decimal,
    /// Aggregated duration in milliseconds of the source run.
    pub duration_ms: u64,
}

/// A condition that must be satisfied for an [`EventTriggerRule`] to fire.
///
/// All conditions on a rule are evaluated with AND semantics: the rule
/// fires only when every condition returns `true`. For OR semantics,
/// create separate rules.
///
/// # Examples
///
/// ```
/// use ironflow_runtime::trigger::event::TriggerCondition;
///
/// let cond = TriggerCondition::Label {
///     key: "env".to_string(),
///     value: "prod".to_string(),
/// };
/// assert!(matches!(cond, TriggerCondition::Label { .. }));
/// ```
pub enum TriggerCondition {
    /// The source run must carry a label with this exact key-value pair.
    Label {
        /// Label key to check.
        key: String,
        /// Expected label value.
        value: String,
    },
    /// An arbitrary predicate evaluated against the [`TriggerContext`].
    ///
    /// Uses [`Arc`] so the condition is `Clone + Send + Sync`.
    Expression(Arc<dyn Fn(&TriggerContext) -> bool + Send + Sync>),
}

impl TriggerCondition {
    /// Evaluate this condition against the given context.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::collections::HashMap;
    /// use ironflow_runtime::trigger::event::{TriggerCondition, TriggerContext};
    /// use rust_decimal::Decimal;
    ///
    /// let ctx = TriggerContext {
    ///     labels: HashMap::from([("env".to_string(), "prod".to_string())]),
    ///     error: None,
    ///     cost_usd: Decimal::ZERO,
    ///     duration_ms: 0,
    /// };
    /// let cond = TriggerCondition::Label {
    ///     key: "env".to_string(),
    ///     value: "prod".to_string(),
    /// };
    /// assert!(cond.evaluate(&ctx));
    /// ```
    ///
    /// # Panics
    ///
    /// Does not panic. If an [`Expression`](TriggerCondition::Expression)
    /// closure panics, the panic is caught and the condition evaluates to
    /// `false`.
    pub fn evaluate(&self, ctx: &TriggerContext) -> bool {
        match self {
            TriggerCondition::Label { key, value } => {
                ctx.labels.get(key).is_some_and(|v| v == value)
            }
            TriggerCondition::Expression(f) => match catch_unwind(AssertUnwindSafe(|| f(ctx))) {
                Ok(result) => result,
                Err(_) => {
                    warn!("expression condition panicked, treating as false");
                    false
                }
            },
        }
    }
}

impl Clone for TriggerCondition {
    fn clone(&self) -> Self {
        match self {
            TriggerCondition::Label { key, value } => TriggerCondition::Label {
                key: key.clone(),
                value: value.clone(),
            },
            TriggerCondition::Expression(f) => TriggerCondition::Expression(Arc::clone(f)),
        }
    }
}

impl fmt::Debug for TriggerCondition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TriggerCondition::Label { key, value } => f
                .debug_struct("Label")
                .field("key", key)
                .field("value", value)
                .finish(),
            TriggerCondition::Expression(_) => f.write_str("Expression(<closure>)"),
        }
    }
}

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
///     conditions: vec![],
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
    /// Optional conditions that must all match for the rule to fire.
    ///
    /// Evaluated with AND semantics. An empty list means the rule fires
    /// unconditionally (backward compatible).
    pub conditions: Vec<TriggerCondition>,
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
///         conditions: vec![],
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
    labels: HashMap<String, String>,
    cost_usd: Decimal,
    duration_ms: u64,
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
    ///         conditions: vec![],
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
    ///         conditions: vec![],
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
                        if rules.is_empty() {
                            continue;
                        }

                        let trigger_ctx = TriggerContext {
                            labels: event.labels.clone(),
                            error: event.error.clone(),
                            cost_usd: event.cost_usd,
                            duration_ms: event.duration_ms,
                        };

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

                            if !rule.conditions.iter().all(|c| c.evaluate(&trigger_ctx)) {
                                info!(
                                    source_workflow = %event.workflow_name,
                                    target_workflow = %rule.target_workflow,
                                    "conditions not met, skipping rule"
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
                    cost_usd,
                    duration_ms,
                    labels,
                    ..
                } => InternalEvent {
                    run_id: *run_id,
                    workflow_name: workflow_name.clone(),
                    event_kind: EventKind::RunFailed,
                    error: error.clone(),
                    labels: labels.clone(),
                    cost_usd: *cost_usd,
                    duration_ms: *duration_ms,
                },
                Event::RunStatusChanged {
                    run_id,
                    workflow_name,
                    error,
                    cost_usd,
                    duration_ms,
                    labels,
                    ..
                } => InternalEvent {
                    run_id: *run_id,
                    workflow_name: workflow_name.clone(),
                    event_kind: EventKind::RunStatusChanged,
                    error: error.clone(),
                    labels: labels.clone(),
                    cost_usd: *cost_usd,
                    duration_ms: *duration_ms,
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
                    labels: HashMap::new(),
                    cost_usd: Decimal::ZERO,
                    duration_ms: 0,
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
                    labels: HashMap::new(),
                    cost_usd: Decimal::ZERO,
                    duration_ms: 0,
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
            conditions: vec![],
        }
    }

    fn internal_event(
        run_id: Uuid,
        workflow_name: &str,
        event_kind: EventKind,
        error: Option<String>,
    ) -> InternalEvent {
        InternalEvent {
            run_id,
            workflow_name: workflow_name.to_string(),
            event_kind,
            error,
            labels: HashMap::new(),
            cost_usd: Decimal::ZERO,
            duration_ms: 0,
        }
    }

    #[tokio::test]
    async fn event_trigger_fires_on_matching_run_failed() {
        let trigger = make_trigger(vec![deploy_to_rollback_rule()]);
        let (sink, mut rx) = TriggerSink::channel(16);
        let token = CancellationToken::new();
        let token_clone = token.clone();

        let run_id = Uuid::now_v7();
        trigger
            .event_tx
            .send(internal_event(
                run_id,
                "deploy",
                EventKind::RunFailed,
                Some("step crashed".to_string()),
            ))
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

        trigger
            .event_tx
            .send(internal_event(
                Uuid::now_v7(),
                "build",
                EventKind::RunFailed,
                None,
            ))
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

        trigger
            .event_tx
            .send(internal_event(
                Uuid::now_v7(),
                "deploy",
                EventKind::RunStatusChanged,
                None,
            ))
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
            .send(internal_event(
                run_id,
                "deploy",
                EventKind::RunFailed,
                Some("timeout".to_string()),
            ))
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
                conditions: vec![],
            },
            EventTriggerRule {
                on_event: EventKind::StepFailed,
                source_workflow: "c".to_string(),
                target_workflow: "d".to_string(),
                max_chain_depth: 3,
                conditions: vec![],
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
            labels: HashMap::new(),
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

    fn internal_event_with_labels(
        run_id: Uuid,
        workflow_name: &str,
        event_kind: EventKind,
        error: Option<String>,
        labels: HashMap<String, String>,
    ) -> InternalEvent {
        InternalEvent {
            run_id,
            workflow_name: workflow_name.to_string(),
            event_kind,
            error,
            labels,
            cost_usd: Decimal::new(42, 2),
            duration_ms: 5000,
        }
    }

    #[tokio::test]
    async fn condition_label_matches() {
        let rule = EventTriggerRule {
            on_event: EventKind::RunFailed,
            source_workflow: "deploy".to_string(),
            target_workflow: "rollback".to_string(),
            max_chain_depth: 3,
            conditions: vec![TriggerCondition::Label {
                key: "env".to_string(),
                value: "prod".to_string(),
            }],
        };
        let trigger = make_trigger(vec![rule]);
        let (sink, mut rx) = TriggerSink::channel(16);
        let token = CancellationToken::new();
        let token_clone = token.clone();

        let run_id = Uuid::now_v7();
        trigger
            .event_tx
            .send(internal_event_with_labels(
                run_id,
                "deploy",
                EventKind::RunFailed,
                Some("crash".to_string()),
                HashMap::from([("env".to_string(), "prod".to_string())]),
            ))
            .await
            .unwrap();

        let handle = tokio::spawn(async move { trigger.start(sink, &token_clone).await });

        let event = timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("timed out")
            .expect("channel closed");

        assert_eq!(event.workflow_name, "rollback");
        token.cancel();
        let _ = handle.await;
    }

    #[tokio::test]
    async fn condition_label_absent_no_fire() {
        let rule = EventTriggerRule {
            on_event: EventKind::RunFailed,
            source_workflow: "deploy".to_string(),
            target_workflow: "rollback".to_string(),
            max_chain_depth: 3,
            conditions: vec![TriggerCondition::Label {
                key: "env".to_string(),
                value: "prod".to_string(),
            }],
        };
        let trigger = make_trigger(vec![rule]);
        let (sink, mut rx) = TriggerSink::channel(16);
        let token = CancellationToken::new();
        let token_clone = token.clone();

        trigger
            .event_tx
            .send(internal_event_with_labels(
                Uuid::now_v7(),
                "deploy",
                EventKind::RunFailed,
                None,
                HashMap::new(),
            ))
            .await
            .unwrap();

        let handle = tokio::spawn(async move { trigger.start(sink, &token_clone).await });

        tokio::time::sleep(Duration::from_millis(100)).await;
        token.cancel();
        let _ = handle.await;

        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn condition_label_wrong_value_no_fire() {
        let rule = EventTriggerRule {
            on_event: EventKind::RunFailed,
            source_workflow: "deploy".to_string(),
            target_workflow: "rollback".to_string(),
            max_chain_depth: 3,
            conditions: vec![TriggerCondition::Label {
                key: "env".to_string(),
                value: "prod".to_string(),
            }],
        };
        let trigger = make_trigger(vec![rule]);
        let (sink, mut rx) = TriggerSink::channel(16);
        let token = CancellationToken::new();
        let token_clone = token.clone();

        trigger
            .event_tx
            .send(internal_event_with_labels(
                Uuid::now_v7(),
                "deploy",
                EventKind::RunFailed,
                None,
                HashMap::from([("env".to_string(), "staging".to_string())]),
            ))
            .await
            .unwrap();

        let handle = tokio::spawn(async move { trigger.start(sink, &token_clone).await });

        tokio::time::sleep(Duration::from_millis(100)).await;
        token.cancel();
        let _ = handle.await;

        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn multiple_conditions_all_match() {
        let rule = EventTriggerRule {
            on_event: EventKind::RunFailed,
            source_workflow: "deploy".to_string(),
            target_workflow: "rollback".to_string(),
            max_chain_depth: 3,
            conditions: vec![
                TriggerCondition::Label {
                    key: "env".to_string(),
                    value: "prod".to_string(),
                },
                TriggerCondition::Label {
                    key: "region".to_string(),
                    value: "eu-west-1".to_string(),
                },
            ],
        };
        let trigger = make_trigger(vec![rule]);
        let (sink, mut rx) = TriggerSink::channel(16);
        let token = CancellationToken::new();
        let token_clone = token.clone();

        trigger
            .event_tx
            .send(internal_event_with_labels(
                Uuid::now_v7(),
                "deploy",
                EventKind::RunFailed,
                None,
                HashMap::from([
                    ("env".to_string(), "prod".to_string()),
                    ("region".to_string(), "eu-west-1".to_string()),
                ]),
            ))
            .await
            .unwrap();

        let handle = tokio::spawn(async move { trigger.start(sink, &token_clone).await });

        let event = timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("timed out")
            .expect("channel closed");

        assert_eq!(event.workflow_name, "rollback");
        token.cancel();
        let _ = handle.await;
    }

    #[tokio::test]
    async fn multiple_conditions_one_fails() {
        let rule = EventTriggerRule {
            on_event: EventKind::RunFailed,
            source_workflow: "deploy".to_string(),
            target_workflow: "rollback".to_string(),
            max_chain_depth: 3,
            conditions: vec![
                TriggerCondition::Label {
                    key: "env".to_string(),
                    value: "prod".to_string(),
                },
                TriggerCondition::Label {
                    key: "region".to_string(),
                    value: "eu-west-1".to_string(),
                },
            ],
        };
        let trigger = make_trigger(vec![rule]);
        let (sink, mut rx) = TriggerSink::channel(16);
        let token = CancellationToken::new();
        let token_clone = token.clone();

        trigger
            .event_tx
            .send(internal_event_with_labels(
                Uuid::now_v7(),
                "deploy",
                EventKind::RunFailed,
                None,
                HashMap::from([("env".to_string(), "prod".to_string())]),
            ))
            .await
            .unwrap();

        let handle = tokio::spawn(async move { trigger.start(sink, &token_clone).await });

        tokio::time::sleep(Duration::from_millis(100)).await;
        token.cancel();
        let _ = handle.await;

        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn empty_conditions_backward_compat() {
        let rule = EventTriggerRule {
            on_event: EventKind::RunFailed,
            source_workflow: "deploy".to_string(),
            target_workflow: "rollback".to_string(),
            max_chain_depth: 3,
            conditions: vec![],
        };
        let trigger = make_trigger(vec![rule]);
        let (sink, mut rx) = TriggerSink::channel(16);
        let token = CancellationToken::new();
        let token_clone = token.clone();

        trigger
            .event_tx
            .send(internal_event(
                Uuid::now_v7(),
                "deploy",
                EventKind::RunFailed,
                Some("boom".to_string()),
            ))
            .await
            .unwrap();

        let handle = tokio::spawn(async move { trigger.start(sink, &token_clone).await });

        let event = timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("timed out")
            .expect("channel closed");

        assert_eq!(event.workflow_name, "rollback");
        token.cancel();
        let _ = handle.await;
    }

    #[tokio::test]
    async fn expression_condition_with_context() {
        let rule = EventTriggerRule {
            on_event: EventKind::RunFailed,
            source_workflow: "deploy".to_string(),
            target_workflow: "rollback".to_string(),
            max_chain_depth: 3,
            conditions: vec![TriggerCondition::Expression(Arc::new(|ctx| {
                ctx.cost_usd > Decimal::new(10, 2) && ctx.duration_ms > 1000
            }))],
        };
        let trigger = make_trigger(vec![rule]);
        let (sink, mut rx) = TriggerSink::channel(16);
        let token = CancellationToken::new();
        let token_clone = token.clone();

        trigger
            .event_tx
            .send(internal_event_with_labels(
                Uuid::now_v7(),
                "deploy",
                EventKind::RunFailed,
                None,
                HashMap::new(),
            ))
            .await
            .unwrap();

        let handle = tokio::spawn(async move { trigger.start(sink, &token_clone).await });

        let event = timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("timed out")
            .expect("channel closed");

        assert_eq!(event.workflow_name, "rollback");
        token.cancel();
        let _ = handle.await;
    }

    #[tokio::test]
    async fn expression_returns_false_no_fire() {
        let rule = EventTriggerRule {
            on_event: EventKind::RunFailed,
            source_workflow: "deploy".to_string(),
            target_workflow: "rollback".to_string(),
            max_chain_depth: 3,
            conditions: vec![TriggerCondition::Expression(Arc::new(|ctx| {
                ctx.cost_usd > Decimal::new(100, 0)
            }))],
        };
        let trigger = make_trigger(vec![rule]);
        let (sink, mut rx) = TriggerSink::channel(16);
        let token = CancellationToken::new();
        let token_clone = token.clone();

        trigger
            .event_tx
            .send(internal_event_with_labels(
                Uuid::now_v7(),
                "deploy",
                EventKind::RunFailed,
                None,
                HashMap::new(),
            ))
            .await
            .unwrap();

        let handle = tokio::spawn(async move { trigger.start(sink, &token_clone).await });

        tokio::time::sleep(Duration::from_millis(100)).await;
        token.cancel();
        let _ = handle.await;

        assert!(rx.try_recv().is_err());
    }
}
