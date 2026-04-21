//! [`AuditLogSubscriber`] -- persists every event to an [`AuditLogStore`].

use std::str::FromStr;
use std::sync::Arc;

use serde_json::to_value;
use tracing::error;
use uuid::Uuid;

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

fn extract_run_id(event: &Event) -> Option<Uuid> {
    match event {
        Event::RunCreated { run_id, .. }
        | Event::RunStatusChanged { run_id, .. }
        | Event::RunFailed { run_id, .. }
        | Event::StepCompleted { run_id, .. }
        | Event::StepFailed { run_id, .. }
        | Event::ApprovalRequested { run_id, .. }
        | Event::ApprovalGranted { run_id, .. }
        | Event::ApprovalRejected { run_id, .. } => Some(*run_id),
        Event::UserSignedIn { .. } | Event::UserSignedUp { .. } | Event::UserSignedOut { .. } => {
            None
        }
    }
}

fn extract_step_id(event: &Event) -> Option<Uuid> {
    match event {
        Event::StepCompleted { step_id, .. }
        | Event::StepFailed { step_id, .. }
        | Event::ApprovalRequested { step_id, .. } => Some(*step_id),
        _ => None,
    }
}

fn extract_user_id(event: &Event) -> Option<Uuid> {
    match event {
        Event::UserSignedIn { user_id, .. }
        | Event::UserSignedUp { user_id, .. }
        | Event::UserSignedOut { user_id, .. } => Some(*user_id),
        _ => None,
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
                run_id: extract_run_id(event),
                step_id: extract_step_id(event),
                user_id: extract_user_id(event),
            };

            if let Err(e) = self.store.append_audit_log(entry).await {
                error!(error = %e, event_type = event.event_type(), "failed to persist audit log entry");
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use chrono::Utc;
    use rust_decimal::Decimal;
    use uuid::Uuid;

    use ironflow_store::audit_log_store::AuditLogStore;
    use ironflow_store::entities::{AuditLogFilter, EventKind};
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::models::RunStatus;

    use super::*;
    use crate::notify::{EventPublisher, EventSubscriber};

    fn sample_run_status_changed() -> Event {
        Event::RunStatusChanged {
            run_id: Uuid::now_v7(),
            workflow_name: "deploy".to_string(),
            from: RunStatus::Running,
            to: RunStatus::Completed,
            error: None,
            cost_usd: Decimal::new(42, 2),
            duration_ms: 5000,
            at: Utc::now(),
        }
    }

    fn sample_user_signed_in() -> Event {
        Event::UserSignedIn {
            user_id: Uuid::now_v7(),
            username: "alice".to_string(),
            at: Utc::now(),
        }
    }

    fn sample_step_failed() -> Event {
        Event::StepFailed {
            run_id: Uuid::now_v7(),
            step_id: Uuid::now_v7(),
            step_name: "build".to_string(),
            kind: ironflow_store::models::StepKind::Shell,
            error: "exit code 1".to_string(),
            at: Utc::now(),
        }
    }

    #[test]
    fn name_is_audit_log() {
        let store = Arc::new(InMemoryStore::new());
        let subscriber = AuditLogSubscriber::new(store);
        assert_eq!(subscriber.name(), "audit_log");
    }

    #[test]
    fn extract_run_id_from_run_event() {
        let event = sample_run_status_changed();
        assert!(extract_run_id(&event).is_some());
    }

    #[test]
    fn extract_run_id_from_user_event_is_none() {
        let event = sample_user_signed_in();
        assert!(extract_run_id(&event).is_none());
    }

    #[test]
    fn extract_step_id_from_step_event() {
        let event = sample_step_failed();
        assert!(extract_step_id(&event).is_some());
    }

    #[test]
    fn extract_step_id_from_run_event_is_none() {
        let event = sample_run_status_changed();
        assert!(extract_step_id(&event).is_none());
    }

    #[test]
    fn extract_user_id_from_user_event() {
        let event = sample_user_signed_in();
        assert!(extract_user_id(&event).is_some());
    }

    #[test]
    fn extract_user_id_from_run_event_is_none() {
        let event = sample_run_status_changed();
        assert!(extract_user_id(&event).is_none());
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
        let event = Event::RunFailed {
            run_id,
            workflow_name: "deploy".to_string(),
            error: Some("step crashed".to_string()),
            cost_usd: Decimal::new(10, 2),
            duration_ms: 3000,
            at: Utc::now(),
        };
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
}
