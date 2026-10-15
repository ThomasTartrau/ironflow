//! [`AuditLogSubscriber`] -- persists every event to an [`AuditLogStore`].

use std::str::FromStr;
use std::sync::Arc;

use serde_json::to_value;
use tracing::error;

use ironflow_store::audit_log_store::AuditLogStore;
use ironflow_store::entities::{EventKind, NewAuditLogEntry};

use super::{Event, EventSubscriber, SubscriberFuture};

/// Subscriber that persists every received event as an audit log entry.
///
/// Extracts contextual IDs (run, step, user) from the event payload
/// for efficient filtering, and stores the full event as JSON.
///
/// # Examples
///
/// ```no_run
/// use std::sync::Arc;
/// use ironflow_engine::notify::{AuditLogSubscriber, Event, EventPublisher};
/// use ironflow_store::memory::InMemoryStore;
///
/// let store = Arc::new(InMemoryStore::new());
/// let mut publisher = EventPublisher::new();
/// publisher.subscribe(
///     AuditLogSubscriber::new(store),
///     Event::ALL,
/// );
/// ```
pub struct AuditLogSubscriber {
    store: Arc<dyn AuditLogStore>,
}

impl AuditLogSubscriber {
    /// Create a new subscriber backed by the given store.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    /// use ironflow_engine::notify::AuditLogSubscriber;
    /// use ironflow_store::memory::InMemoryStore;
    ///
    /// let store = Arc::new(InMemoryStore::new());
    /// let subscriber = AuditLogSubscriber::new(store);
    /// ```
    pub fn new(store: Arc<dyn AuditLogStore>) -> Self {
        Self { store }
    }
}

impl EventSubscriber for AuditLogSubscriber {
    fn name(&self) -> &str {
        "audit_log"
    }

    fn handle<'a>(&'a self, event: &'a Event) -> SubscriberFuture<'a> {
        Box::pin(async move {
            let event_kind = match EventKind::from_str(event.event_type()) {
                Ok(k) => k,
                Err(e) => {
                    error!(error = %e, event_type = event.event_type(), "unknown event kind for audit log");
                    return;
                }
            };

            let payload = match to_value(event) {
                Ok(v) => v,
                Err(e) => {
                    error!(error = %e, event_type = event.event_type(), "failed to serialize event for audit log");
                    return;
                }
            };

            let entry = NewAuditLogEntry {
                event_type: event_kind,
                payload,
                run_id: event.run_id(),
                step_id: event.step_id(),
                user_id: event.user_id(),
            };

            if let Err(e) = self.store.append_audit_log(entry).await {
                error!(error = %e, event_type = event.event_type(), "failed to persist audit log entry");
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::Duration;

    use chrono::Utc;
    use rust_decimal::Decimal;
    use uuid::Uuid;

    use ironflow_store::audit_log_store::AuditLogStore;
    use ironflow_store::entities::{AuditLogEntry, AuditLogFilter, EventKind};
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::models::{RunStatus, StepKind};

    use super::*;
    use crate::notify::{
        ApprovalGrantedEvent, ApprovalRejectedEvent, EventPublisher, EventSubscriber,
        RunFailedEvent, RunStatusChangedEvent, StepFailedEvent, UserSignedInEvent,
        WorkflowPausedEvent, WorkflowResumedEvent,
    };

    fn sample_run_status_changed() -> Event {
        Event::RunStatusChanged(RunStatusChangedEvent {
            run_id: Uuid::now_v7(),
            workflow_name: "deploy".to_string(),
            from: RunStatus::Running,
            to: RunStatus::Completed,
            error: None,
            cost_usd: Decimal::new(42, 2),
            duration_ms: 5000,
            labels: HashMap::new(),
            at: Utc::now(),
        })
    }

    fn sample_user_signed_in() -> Event {
        Event::UserSignedIn(UserSignedInEvent {
            user_id: Uuid::now_v7(),
            username: "alice".to_string(),
            at: Utc::now(),
        })
    }

    fn sample_step_failed() -> Event {
        Event::StepFailed(StepFailedEvent {
            run_id: Uuid::now_v7(),
            step_id: Uuid::now_v7(),
            step_name: "build".to_string(),
            kind: StepKind::Shell,
            error: "exit code 1".to_string(),
            at: Utc::now(),
        })
    }

    #[test]
    fn name_is_audit_log() {
        let store = Arc::new(InMemoryStore::new());
        let subscriber = AuditLogSubscriber::new(store);
        assert_eq!(subscriber.name(), "audit_log");
    }

    #[test]
    fn run_id_from_run_event() {
        let event = sample_run_status_changed();
        assert!(event.run_id().is_some());
    }

    #[test]
    fn run_id_from_user_event_is_none() {
        let event = sample_user_signed_in();
        assert!(event.run_id().is_none());
    }

    #[test]
    fn step_id_from_step_event() {
        let event = sample_step_failed();
        assert!(event.step_id().is_some());
    }

    #[test]
    fn step_id_from_run_event_is_none() {
        let event = sample_run_status_changed();
        assert!(event.step_id().is_none());
    }

    #[test]
    fn user_id_from_user_event() {
        let event = sample_user_signed_in();
        assert!(event.user_id().is_some());
    }

    #[test]
    fn user_id_from_run_event_is_none() {
        let event = sample_run_status_changed();
        assert!(event.user_id().is_none());
    }

    #[tokio::test]
    async fn handle_persists_event() {
        let store = Arc::new(InMemoryStore::new());
        let subscriber = AuditLogSubscriber::new(store.clone());

        let event = sample_run_status_changed();
        subscriber.handle(&event).await;

        let page = store
            .list_audit_logs(AuditLogFilter::default(), 1, 20)
            .await
            .unwrap();

        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].event_type, EventKind::RunStatusChanged);
        assert!(page.items[0].run_id.is_some());
        assert!(page.items[0].step_id.is_none());
        assert!(page.items[0].user_id.is_none());
    }

    #[tokio::test]
    async fn handle_persists_step_event_with_ids() {
        let store = Arc::new(InMemoryStore::new());
        let subscriber = AuditLogSubscriber::new(store.clone());

        let event = sample_step_failed();
        subscriber.handle(&event).await;

        let page = store
            .list_audit_logs(AuditLogFilter::default(), 1, 20)
            .await
            .unwrap();

        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].event_type, EventKind::StepFailed);
        assert!(page.items[0].run_id.is_some());
        assert!(page.items[0].step_id.is_some());
    }

    #[tokio::test]
    async fn handle_persists_user_event_with_user_id() {
        let store = Arc::new(InMemoryStore::new());
        let subscriber = AuditLogSubscriber::new(store.clone());

        let event = sample_user_signed_in();
        subscriber.handle(&event).await;

        let page = store
            .list_audit_logs(AuditLogFilter::default(), 1, 20)
            .await
            .unwrap();

        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].event_type, EventKind::UserSignedIn);
        assert!(page.items[0].user_id.is_some());
        assert!(page.items[0].run_id.is_none());
    }

    #[tokio::test]
    async fn publisher_dispatches_to_audit_log_subscriber() {
        let store = Arc::new(InMemoryStore::new());
        let mut publisher = EventPublisher::new();

        publisher.subscribe(AuditLogSubscriber::new(store.clone()), Event::ALL);

        publisher.publish(sample_run_status_changed());
        publisher.publish(sample_user_signed_in());
        publisher.publish(sample_step_failed());

        tokio::time::sleep(Duration::from_millis(100)).await;

        let page = store
            .list_audit_logs(AuditLogFilter::default(), 1, 20)
            .await
            .unwrap();

        assert_eq!(page.items.len(), 3);
    }

    #[tokio::test]
    async fn full_event_payload_is_preserved() {
        let store = Arc::new(InMemoryStore::new());
        let subscriber = AuditLogSubscriber::new(store.clone());

        let run_id = Uuid::now_v7();
        let event = Event::RunFailed(RunFailedEvent {
            run_id,
            workflow_name: "deploy".to_string(),
            error: Some("step crashed".to_string()),
            cost_usd: Decimal::new(10, 2),
            duration_ms: 3000,
            labels: HashMap::new(),
            at: Utc::now(),
        });
        subscriber.handle(&event).await;

        let page = store
            .list_audit_logs(AuditLogFilter::default(), 1, 20)
            .await
            .unwrap();

        let payload = &page.items[0].payload;
        assert_eq!(payload["type"], "run_failed");
        assert_eq!(payload["workflow_name"], "deploy");
        assert_eq!(payload["error"], "step crashed");
    }

    /// Persist `event` and return the single entry it produced.
    async fn persist_one(event: &Event) -> AuditLogEntry {
        let store = Arc::new(InMemoryStore::new());
        let subscriber = AuditLogSubscriber::new(store.clone());
        subscriber.handle(event).await;

        let mut page = store
            .list_audit_logs(AuditLogFilter::default(), 1, 20)
            .await
            .unwrap();
        assert_eq!(page.items.len(), 1);
        page.items.remove(0)
    }

    #[tokio::test]
    async fn audit_persists_approval_granted_with_user_id() {
        let user_id = Uuid::now_v7();
        let run_id = Uuid::now_v7();
        let entry = persist_one(&Event::ApprovalGranted(ApprovalGrantedEvent {
            run_id,
            step_id: Some(Uuid::now_v7()),
            approved_by: "ci-key".to_string(),
            user_id: Some(user_id),
            approvals_received: 1,
            approvals_required: 1,
            requirement: None,
            at: Utc::now(),
        }))
        .await;

        assert_eq!(entry.event_type, EventKind::ApprovalGranted);
        assert_eq!(entry.user_id, Some(user_id));
        assert_eq!(entry.run_id, Some(run_id));
        assert_eq!(entry.payload["user_id"], user_id.to_string());
        assert_eq!(entry.payload["approved_by"], "ci-key");
    }

    #[tokio::test]
    async fn audit_persists_approval_rejected_with_user_id() {
        let user_id = Uuid::now_v7();
        let entry = persist_one(&Event::ApprovalRejected(ApprovalRejectedEvent {
            run_id: Uuid::now_v7(),
            step_id: Some(Uuid::now_v7()),
            rejected_by: "bob".to_string(),
            user_id: Some(user_id),
            reason: Some("wrong environment".to_string()),
            requirement: None,
            at: Utc::now(),
        }))
        .await;

        assert_eq!(entry.event_type, EventKind::ApprovalRejected);
        assert_eq!(entry.user_id, Some(user_id));
        assert_eq!(entry.payload["reason"], "wrong environment");
    }

    #[tokio::test]
    async fn audit_persists_workflow_paused_with_user_id() {
        let user_id = Uuid::now_v7();
        let entry = persist_one(&Event::WorkflowPaused(WorkflowPausedEvent {
            workflow_name: "deploy".to_string(),
            user_id: Some(user_id),
            at: Utc::now(),
        }))
        .await;

        assert_eq!(entry.event_type, EventKind::WorkflowPaused);
        assert_eq!(entry.user_id, Some(user_id));
        assert!(entry.run_id.is_none());
        assert_eq!(entry.payload["workflow_name"], "deploy");
    }

    #[tokio::test]
    async fn audit_persists_workflow_resumed_with_user_id() {
        let user_id = Uuid::now_v7();
        let entry = persist_one(&Event::WorkflowResumed(WorkflowResumedEvent {
            workflow_name: "deploy".to_string(),
            user_id: Some(user_id),
            at: Utc::now(),
        }))
        .await;

        assert_eq!(entry.event_type, EventKind::WorkflowResumed);
        assert_eq!(entry.user_id, Some(user_id));
        assert_eq!(entry.payload["workflow_name"], "deploy");
    }

    #[tokio::test]
    async fn audit_system_decision_has_no_user_id() {
        let entry = persist_one(&Event::ApprovalRejected(ApprovalRejectedEvent {
            run_id: Uuid::now_v7(),
            step_id: Some(Uuid::now_v7()),
            rejected_by: "system:timeout".to_string(),
            user_id: None,
            reason: None,
            requirement: None,
            at: Utc::now(),
        }))
        .await;

        assert_eq!(entry.event_type, EventKind::ApprovalRejected);
        assert!(entry.user_id.is_none());
        assert_eq!(entry.payload["rejected_by"], "system:timeout");
    }
}
