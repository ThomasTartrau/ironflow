//! [`EventPublisher`] -- broadcasts events to filtered subscribers.

use std::sync::Arc;

use super::{Event, EventSubscriber};

/// A subscriber paired with its event type filter.
struct Subscription {
    subscriber: Arc<dyn EventSubscriber>,
    event_types: Vec<&'static str>,
}

impl Subscription {
    /// Returns `true` if this subscription accepts the given event.
    fn accepts(&self, event: &Event) -> bool {
        self.event_types.contains(&event.event_type())
    }
}

/// Broadcasts [`Event`]s to registered [`EventSubscriber`]s.
///
/// Each subscriber is paired with an event type filter at subscription
/// time. Only matching events are dispatched. Each call runs in a
/// spawned task so that slow subscribers do not block the engine.
///
/// # Examples
///
/// ```no_run
/// use ironflow_engine::notify::{EventPublisher, WebhookSubscriber, Event};
///
/// let mut publisher = EventPublisher::new();
/// publisher.subscribe(
///     WebhookSubscriber::new("https://hooks.example.com/events"),
///     &[Event::RUN_STATUS_CHANGED, Event::STEP_FAILED],
/// );
/// ```
pub struct EventPublisher {
    subscriptions: Vec<Subscription>,
}

impl EventPublisher {
    /// Create an empty publisher with no subscribers.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::notify::EventPublisher;
    ///
    /// let publisher = EventPublisher::new();
    /// assert_eq!(publisher.subscriber_count(), 0);
    /// ```
    pub fn new() -> Self {
        Self {
            subscriptions: Vec::new(),
        }
    }

    /// Register a subscriber with an event type filter.
    ///
    /// The subscriber is called only for events whose
    /// [`event_type()`](Event::event_type) is in `event_types`.
    /// Pass [`Event::ALL`] to receive every event.
    ///
    /// Use the `Event::*` constants for the filter values.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::notify::{EventPublisher, WebhookSubscriber, Event};
    ///
    /// let mut publisher = EventPublisher::new();
    ///
    /// // Only on specific event types:
    /// publisher.subscribe(
    ///     WebhookSubscriber::new("https://example.com/hook"),
    ///     &[Event::RUN_STATUS_CHANGED, Event::STEP_FAILED],
    /// );
    ///
    /// // On all events:
    /// publisher.subscribe(
    ///     WebhookSubscriber::new("https://example.com/all"),
    ///     Event::ALL,
    /// );
    /// ```
    pub fn subscribe(
        &mut self,
        subscriber: impl EventSubscriber + 'static,
        event_types: &[&'static str],
    ) {
        self.subscriptions.push(Subscription {
            subscriber: Arc::new(subscriber),
            event_types: event_types.to_vec(),
        });
    }

    /// Number of registered subscribers.
    pub fn subscriber_count(&self) -> usize {
        self.subscriptions.len()
    }

    /// Broadcast an event to all matching subscribers.
    ///
    /// Each matching subscriber runs in its own spawned task. This
    /// method returns immediately and never blocks.
    pub fn publish(&self, event: Event) {
        for subscription in &self.subscriptions {
            if !subscription.accepts(&event) {
                continue;
            }
            let subscriber = subscription.subscriber.clone();
            let event = event.clone();
            tokio::spawn(async move {
                subscriber.handle(&event).await;
            });
        }
    }
}

impl Default for EventPublisher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notify::{SubscriberFuture, WebhookSubscriber};
    use rust_decimal::Decimal;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

    use chrono::Utc;
    use ironflow_store::models::RunStatus;
    use uuid::Uuid;

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

    #[test]
    fn starts_empty() {
        let publisher = EventPublisher::new();
        assert_eq!(publisher.subscriber_count(), 0);
    }

    #[test]
    fn subscribe_increments_count() {
        let mut publisher = EventPublisher::new();
        publisher.subscribe(
            WebhookSubscriber::new("https://example.com"),
            &[Event::RUN_STATUS_CHANGED],
        );
        assert_eq!(publisher.subscriber_count(), 1);
    }

    #[test]
    fn publish_with_no_subscribers_is_noop() {
        let publisher = EventPublisher::new();
        publisher.publish(sample_run_status_changed());
    }

    #[test]
    fn default_is_empty() {
        let publisher = EventPublisher::default();
        assert_eq!(publisher.subscriber_count(), 0);
    }

    struct CountingSubscriber {
        count: AtomicU32,
    }

    impl CountingSubscriber {
        fn new() -> Self {
            Self {
                count: AtomicU32::new(0),
            }
        }

        fn count(&self) -> u32 {
            self.count.load(Ordering::SeqCst)
        }
    }

    impl EventSubscriber for CountingSubscriber {
        fn name(&self) -> &str {
            "counting"
        }

        fn handle<'a>(&'a self, _event: &'a Event) -> SubscriberFuture<'a> {
            Box::pin(async move {
                self.count.fetch_add(1, Ordering::SeqCst);
            })
        }
    }

    #[tokio::test]
    async fn subscriber_receives_matching_events() {
        let subscriber = Arc::new(CountingSubscriber::new());
        let mut publisher = EventPublisher::new();

        struct ArcSub(Arc<CountingSubscriber>);
        impl EventSubscriber for ArcSub {
            fn name(&self) -> &str {
                self.0.name()
            }
            fn handle<'a>(&'a self, event: &'a Event) -> SubscriberFuture<'a> {
                self.0.handle(event)
            }
        }

        publisher.subscribe(ArcSub(subscriber.clone()), &[Event::RUN_STATUS_CHANGED]);

        publisher.publish(sample_run_status_changed()); // matches
        publisher.publish(sample_user_signed_in()); // filtered out

        tokio::time::sleep(Duration::from_millis(50)).await;

        assert_eq!(subscriber.count(), 1);
    }

    #[tokio::test]
    async fn all_filter_matches_everything() {
        let subscriber = Arc::new(CountingSubscriber::new());
        let mut publisher = EventPublisher::new();

        struct ArcSub(Arc<CountingSubscriber>);
        impl EventSubscriber for ArcSub {
            fn name(&self) -> &str {
                self.0.name()
            }
            fn handle<'a>(&'a self, event: &'a Event) -> SubscriberFuture<'a> {
                self.0.handle(event)
            }
        }

        publisher.subscribe(ArcSub(subscriber.clone()), Event::ALL);

        publisher.publish(sample_run_status_changed());
        publisher.publish(sample_user_signed_in());

        tokio::time::sleep(Duration::from_millis(50)).await;

        assert_eq!(subscriber.count(), 2);
    }

    #[tokio::test]
    async fn empty_filter_matches_nothing() {
        let subscriber = Arc::new(CountingSubscriber::new());
        let mut publisher = EventPublisher::new();

        struct ArcSub(Arc<CountingSubscriber>);
        impl EventSubscriber for ArcSub {
            fn name(&self) -> &str {
                self.0.name()
            }
            fn handle<'a>(&'a self, event: &'a Event) -> SubscriberFuture<'a> {
                self.0.handle(event)
            }
        }

        publisher.subscribe(ArcSub(subscriber.clone()), &[]);

        publisher.publish(sample_run_status_changed());
        publisher.publish(sample_user_signed_in());

        tokio::time::sleep(Duration::from_millis(50)).await;

        assert_eq!(subscriber.count(), 0);
    }
}
