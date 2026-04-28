//! [`EventSubscriber`] trait -- react to domain events.

use std::future::Future;
use std::pin::Pin;

use super::Event;

/// Boxed future returned by [`EventSubscriber::handle`].
pub type SubscriberFuture<'a> = Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

/// A subscriber that reacts to domain events.
///
/// Implement this trait to create custom notification channels (Slack,
/// Discord, PagerDuty, etc.). The engine broadcasts events to all
/// registered subscribers via [`EventPublisher`](super::EventPublisher).
///
/// # Contract
///
/// - [`handle`](EventSubscriber::handle) is called only for events that
///   match the filter configured at subscription time.
/// - Implementations must not block -- heavy work should be spawned.
/// - Errors are logged internally; they must not propagate.
///
/// # Examples
///
/// ```no_run
/// use ironflow_engine::notify::{EventSubscriber, Event, SubscriberFuture};
///
/// struct LogSubscriber;
///
/// impl EventSubscriber for LogSubscriber {
///     fn name(&self) -> &str { "log" }
///
///     fn handle<'a>(&'a self, event: &'a Event) -> SubscriberFuture<'a> {
///         Box::pin(async move {
///             println!("[{}] {:?}", event.event_type(), event);
///         })
///     }
/// }
/// ```
pub trait EventSubscriber: Send + Sync {
    /// A short identifier for this subscriber (used in logs).
    fn name(&self) -> &str;

    /// Handle a domain event.
    ///
    /// Only called for events matching the filter set at subscription
    /// time. The subscriber does not need to filter.
    fn handle<'a>(&'a self, event: &'a Event) -> SubscriberFuture<'a>;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestSubscriber {
        name: String,
    }

    impl EventSubscriber for TestSubscriber {
        fn name(&self) -> &str {
            &self.name
        }

        fn handle<'a>(&'a self, _event: &'a Event) -> SubscriberFuture<'a> {
            Box::pin(async move {
                // No-op for testing
            })
        }
    }

    struct CountingSubscriber {
        name: String,
    }

    impl EventSubscriber for CountingSubscriber {
        fn name(&self) -> &str {
            &self.name
        }

        fn handle<'a>(&'a self, _event: &'a Event) -> SubscriberFuture<'a> {
            Box::pin(async move {
                // Simulates async work
                tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
            })
        }
    }

    #[test]
    fn subscriber_has_identifier_name() {
        let sub = TestSubscriber {
            name: "test_sub".to_string(),
        };
        assert_eq!(sub.name(), "test_sub");
    }

    #[test]
    fn subscriber_name_is_consistent() {
        let sub = TestSubscriber {
            name: "my_subscriber".to_string(),
        };
        assert_eq!(sub.name(), "my_subscriber");
        assert_eq!(sub.name(), "my_subscriber");
    }

    #[test]
    fn different_subscribers_have_different_names() {
        let sub1 = TestSubscriber {
            name: "sub1".to_string(),
        };
        let sub2 = TestSubscriber {
            name: "sub2".to_string(),
        };

        assert_ne!(sub1.name(), sub2.name());
    }

    #[tokio::test]
    async fn subscriber_handle_completes_successfully() {
        use chrono::Utc;
        let sub = TestSubscriber {
            name: "test".to_string(),
        };

        // Create a dummy event for testing
        let event = Event::RunCreated {
            run_id: uuid::Uuid::now_v7(),
            workflow_name: "test-wf".to_string(),
            at: Utc::now(),
        };

        // Should complete without error
        sub.handle(&event).await;
    }

    #[tokio::test]
    async fn subscriber_handle_is_async() {
        use chrono::Utc;
        let sub = CountingSubscriber {
            name: "async_test".to_string(),
        };

        let event = Event::RunCreated {
            run_id: uuid::Uuid::now_v7(),
            workflow_name: "test".to_string(),
            at: Utc::now(),
        };

        let start = std::time::Instant::now();
        sub.handle(&event).await;
        let elapsed = start.elapsed();

        // Should have taken at least 1ms due to the sleep
        assert!(elapsed.as_millis() >= 1);
    }

    #[tokio::test]
    async fn multiple_subscribers_can_handle_same_event() {
        use chrono::Utc;
        let sub1 = TestSubscriber {
            name: "sub1".to_string(),
        };
        let sub2 = TestSubscriber {
            name: "sub2".to_string(),
        };

        let event = Event::RunCreated {
            run_id: uuid::Uuid::now_v7(),
            workflow_name: "test".to_string(),
            at: Utc::now(),
        };

        // Both should handle without issue
        sub1.handle(&event).await;
        sub2.handle(&event).await;
    }

    #[test]
    fn subscriber_implements_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<TestSubscriber>();
        assert_send_sync::<CountingSubscriber>();
    }

    #[tokio::test]
    async fn subscriber_future_is_boxed() {
        use chrono::Utc;
        let sub = TestSubscriber {
            name: "boxed_test".to_string(),
        };

        let event = Event::RunCreated {
            run_id: uuid::Uuid::now_v7(),
            workflow_name: "test".to_string(),
            at: Utc::now(),
        };

        let future = sub.handle(&event);
        // The future should be a Pin<Box<_>> and awaitable
        let _ = future.await;
    }

    #[test]
    fn subscriber_name_borrowed_lifetime() {
        let sub = TestSubscriber {
            name: "lifetime_test".to_string(),
        };

        let name1 = sub.name();
        let name2 = sub.name();

        // Both should be valid references
        assert_eq!(name1, name2);
        assert_eq!(name1, "lifetime_test");
    }
}
