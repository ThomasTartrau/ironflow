//! Event-driven notification system for the ironflow lifecycle.
//!
//! Uses a publisher/subscriber pattern: the [`EventPublisher`] broadcasts
//! [`Event`]s to registered [`EventSubscriber`]s. Event type filtering
//! is handled by the publisher at subscription time -- subscribers only
//! receive events they signed up for.
//!
//! Built-in subscribers:
//! - [`WebhookSubscriber`] -- POSTs JSON to a URL with retry and exponential backoff.
//! - [`BetterStackSubscriber`] -- forwards error events to BetterStack Logs.
//!
//! # Architecture
//!
//! - [`Event`] -- domain event enum covering runs, steps, approvals, auth.
//! - [`EventSubscriber`] -- trait: receives and handles a single event.
//! - [`EventPublisher`] -- holds subscriptions (subscriber + event type filter),
//!   dispatches matching events via `tokio::spawn`.
//! - [`WebhookSubscriber`] -- built-in HTTP POST implementation.
//!
//! # Examples
//!
//! ```no_run
//! use ironflow_engine::notify::{Event, EventPublisher, WebhookSubscriber};
//!
//! let mut publisher = EventPublisher::new();
//! publisher.subscribe(
//!     WebhookSubscriber::new("https://hooks.example.com/events"),
//!     &[Event::RUN_STATUS_CHANGED, Event::STEP_FAILED],
//! );
//! ```

mod betterstack;
mod event;
mod publisher;
mod subscriber;
mod webhook;

pub use betterstack::BetterStackSubscriber;
pub use event::Event;
pub use publisher::EventPublisher;
pub use subscriber::{EventSubscriber, SubscriberFuture};
pub use webhook::WebhookSubscriber;
