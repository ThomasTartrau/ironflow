//! Server-Sent Events broadcaster for real-time event streaming.
//!
//! [`SseBroadcaster`] implements [`EventSubscriber`] and forwards events into
//! a [`tokio::sync::broadcast`] channel. The SSE route creates receivers from
//! this channel and streams them to connected clients.

use tokio::sync::broadcast;

use ironflow_engine::notify::{Event, EventSubscriber, SubscriberFuture};

/// Default broadcast channel capacity.
const DEFAULT_CAPACITY: usize = 256;

/// Broadcasts [`Event`]s to SSE clients via a [`tokio::sync::broadcast`] channel.
///
/// Register this as an [`EventSubscriber`] on the [`Engine`](ironflow_engine::engine::Engine)
/// to forward all (or filtered) domain events to connected SSE clients.
///
/// # Examples
///
/// ```
/// use ironflow_api::sse::SseBroadcaster;
///
/// let broadcaster = SseBroadcaster::new();
/// assert_eq!(broadcaster.receiver_count(), 0);
/// let _receiver = broadcaster.subscribe();
/// assert_eq!(broadcaster.receiver_count(), 1);
/// ```
pub struct SseBroadcaster {
    sender: broadcast::Sender<Event>,
}

impl SseBroadcaster {
    /// Create a new broadcaster with the default capacity (256).
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_api::sse::SseBroadcaster;
    ///
    /// let broadcaster = SseBroadcaster::new();
    /// ```
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_CAPACITY)
    }

    /// Create a new broadcaster with a custom channel capacity.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_api::sse::SseBroadcaster;
    ///
    /// let broadcaster = SseBroadcaster::with_capacity(64);
    /// ```
    pub fn with_capacity(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }

    /// Create a new receiver for the broadcast channel.
    ///
    /// Each SSE client connection calls this to get its own receiver.
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.sender.subscribe()
    }

    /// Returns the number of active receivers (connected SSE clients).
    pub fn receiver_count(&self) -> usize {
        self.sender.receiver_count()
    }

    /// Returns a clone of the underlying sender.
    ///
    /// Stored in [`AppState`](crate::state::AppState) so that the SSE route
    /// can create receivers without holding a reference to the broadcaster.
    pub fn sender(&self) -> broadcast::Sender<Event> {
        self.sender.clone()
    }
}

impl Default for SseBroadcaster {
    fn default() -> Self {
        Self::new()
    }
}

impl EventSubscriber for SseBroadcaster {
    fn name(&self) -> &str {
        "sse"
    }

    fn handle<'a>(&'a self, event: &'a Event) -> SubscriberFuture<'a> {
        let event = event.clone();
        Box::pin(async move {
            // Ignore send errors: they mean no receivers are connected.
            let _ = self.sender.send(event);
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use chrono::Utc;
    use ironflow_store::models::RunStatus;
    use rust_decimal::Decimal;
    use uuid::Uuid;

    fn sample_event() -> Event {
        Event::RunStatusChanged {
            run_id: Uuid::now_v7(),
            workflow_name: "deploy".to_string(),
            from: RunStatus::Running,
            to: RunStatus::Completed,
            error: None,
            cost_usd: Decimal::ZERO,
            duration_ms: 1000,
            labels: HashMap::new(),
            at: Utc::now(),
        }
    }

    #[test]
    fn new_creates_broadcaster() {
        let broadcaster = SseBroadcaster::new();
        assert_eq!(broadcaster.receiver_count(), 0);
    }

    #[test]
    fn default_creates_broadcaster() {
        let broadcaster = SseBroadcaster::default();
        assert_eq!(broadcaster.receiver_count(), 0);
    }

    #[test]
    fn subscribe_creates_receiver() {
        let broadcaster = SseBroadcaster::new();
        let _rx = broadcaster.subscribe();
        assert_eq!(broadcaster.receiver_count(), 1);
    }

    #[test]
    fn receiver_count_tracks_active_receivers() {
        let broadcaster = SseBroadcaster::new();
        let _rx1 = broadcaster.subscribe();
        let _rx2 = broadcaster.subscribe();
        assert_eq!(broadcaster.receiver_count(), 2);
        drop(_rx1);
        assert_eq!(broadcaster.receiver_count(), 1);
    }

    #[tokio::test]
    async fn handle_sends_event_to_receivers() {
        let broadcaster = SseBroadcaster::new();
        let mut rx = broadcaster.subscribe();

        let event = sample_event();
        broadcaster.handle(&event).await;

        let received = rx.recv().await.expect("should receive event");
        assert_eq!(received.event_type(), "run_status_changed");
    }

    #[tokio::test]
    async fn handle_no_receivers_does_not_panic() {
        let broadcaster = SseBroadcaster::new();
        let event = sample_event();
        // No receivers -- should not panic
        broadcaster.handle(&event).await;
    }

    #[test]
    fn sender_returns_clone() {
        let broadcaster = SseBroadcaster::new();
        let sender = broadcaster.sender();
        let _rx = sender.subscribe();
        // The receiver from the cloned sender counts on the original channel
        assert_eq!(broadcaster.receiver_count(), 1);
    }

    #[test]
    fn name_returns_sse() {
        let broadcaster = SseBroadcaster::new();
        assert_eq!(broadcaster.name(), "sse");
    }
}
