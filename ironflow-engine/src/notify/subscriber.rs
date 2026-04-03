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
