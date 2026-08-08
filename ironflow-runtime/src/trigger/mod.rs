//! Pluggable trigger sources for starting workflow runs.
//!
//! A [`Trigger`] listens for external or internal signals and emits
//! [`TriggerEvent`]s through a [`TriggerSink`]. The runtime starts all
//! registered triggers alongside the HTTP server and cron scheduler, and
//! forwards their events to the configured handler.
//!
//! Built-in triggers:
//! - `EventTrigger` -- reacts to internal domain events (workflow chaining).
//! - `NatsTrigger` -- consumes messages from a NATS JetStream subject
//!   (behind the `trigger-nats` feature flag).
//!
//! # Examples
//!
//! ```no_run
//! use ironflow_runtime::trigger::{Trigger, TriggerSink, TriggerEvent};
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

pub mod event;
#[cfg(feature = "trigger-nats")]
pub mod nats;

use std::future::Future;
use std::pin::Pin;

use serde_json::Value;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use ironflow_store::entities::TriggerKind;

/// Future returned by [`Trigger::start`].
pub type TriggerFuture<'a> = Pin<Box<dyn Future<Output = Result<(), TriggerError>> + Send + 'a>>;

/// A source of workflow run triggers.
///
/// Implementations listen for some signal (domain event, message queue,
/// external webhook, etc.) and send [`TriggerEvent`]s through the
/// [`TriggerSink`] to request run creation.
///
/// # Lifecycle
///
/// 1. The runtime calls [`start`](Trigger::start) with a sink and a
///    cancellation token.
/// 2. The trigger runs until the token is cancelled (graceful shutdown)
///    or an unrecoverable error occurs.
/// 3. Dropping the trigger releases all resources.
///
/// # Examples
///
/// ```no_run
/// use ironflow_runtime::trigger::{Trigger, TriggerFuture, TriggerSink, TriggerError};
/// use tokio_util::sync::CancellationToken;
///
/// struct MyTrigger;
///
/// impl Trigger for MyTrigger {
///     fn name(&self) -> &str { "my-trigger" }
///
///     fn start<'a>(
///         &'a self,
///         _sink: TriggerSink,
///         token: &'a CancellationToken,
///     ) -> TriggerFuture<'a> {
///         Box::pin(async move {
///             token.cancelled().await;
///             Ok(())
///         })
///     }
/// }
/// ```
pub trait Trigger: Send + Sync {
    /// Human-readable name for logging.
    fn name(&self) -> &str;

    /// Start the trigger.
    ///
    /// The implementation should send [`TriggerEvent`]s through `sink`
    /// whenever a run should be created, and return when `token` is
    /// cancelled.
    fn start<'a>(&'a self, sink: TriggerSink, token: &'a CancellationToken) -> TriggerFuture<'a>;
}

/// Error type for trigger operations.
///
/// # Examples
///
/// ```
/// use ironflow_runtime::trigger::TriggerError;
///
/// let err = TriggerError::Failed("connection lost".to_string());
/// assert!(err.to_string().contains("connection lost"));
/// ```
#[derive(Debug, thiserror::Error)]
pub enum TriggerError {
    /// A trigger encountered an unrecoverable error.
    #[error("trigger failed: {0}")]
    Failed(String),
}

/// A request to create a workflow run, emitted by a [`Trigger`].
///
/// # Examples
///
/// ```
/// use ironflow_runtime::trigger::TriggerEvent;
/// use ironflow_store::entities::TriggerKind;
/// use serde_json::json;
///
/// let event = TriggerEvent {
///     workflow_name: "rollback".to_string(),
///     payload: json!({"source": "deploy"}),
///     trigger_kind: TriggerKind::Manual,
/// };
/// assert_eq!(event.workflow_name, "rollback");
/// ```
#[derive(Debug, Clone)]
pub struct TriggerEvent {
    /// The workflow to run.
    pub workflow_name: String,
    /// Payload passed to the new run.
    pub payload: Value,
    /// How the run was triggered (stored on the run record).
    pub trigger_kind: TriggerKind,
}

/// Channel endpoint for triggers to emit [`TriggerEvent`]s.
///
/// Obtained from [`TriggerSink::channel`] and passed to
/// [`Trigger::start`].
///
/// # Examples
///
/// ```
/// use ironflow_runtime::trigger::{TriggerSink, TriggerEvent};
/// use ironflow_store::entities::TriggerKind;
/// use serde_json::json;
///
/// let (sink, mut rx) = TriggerSink::channel(16);
///
/// # tokio_test::block_on(async {
/// sink.send(TriggerEvent {
///     workflow_name: "deploy".to_string(),
///     payload: json!({}),
///     trigger_kind: TriggerKind::Manual,
/// }).await.unwrap();
///
/// let event = rx.recv().await.unwrap();
/// assert_eq!(event.workflow_name, "deploy");
/// # });
/// ```
#[derive(Clone)]
pub struct TriggerSink {
    tx: mpsc::Sender<TriggerEvent>,
}

impl TriggerSink {
    /// Create a sink/receiver pair with the given buffer capacity.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_runtime::trigger::TriggerSink;
    ///
    /// let (sink, rx) = TriggerSink::channel(32);
    /// drop(rx);
    /// ```
    pub fn channel(buffer: usize) -> (Self, mpsc::Receiver<TriggerEvent>) {
        let (tx, rx) = mpsc::channel(buffer);
        (Self { tx }, rx)
    }

    /// Send a trigger event.
    ///
    /// # Errors
    ///
    /// Returns an error if the receiver has been dropped.
    pub async fn send(&self, event: TriggerEvent) -> Result<(), TriggerError> {
        self.tx
            .send(event)
            .await
            .map_err(|e| TriggerError::Failed(format!("sink closed: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn trigger_error_display() {
        let err = TriggerError::Failed("boom".to_string());
        assert_eq!(err.to_string(), "trigger failed: boom");
    }

    #[tokio::test]
    async fn sink_sends_and_receives() {
        let (sink, mut rx) = TriggerSink::channel(4);
        sink.send(TriggerEvent {
            workflow_name: "deploy".to_string(),
            payload: json!({"key": "val"}),
            trigger_kind: TriggerKind::Manual,
        })
        .await
        .unwrap();

        let event = rx.recv().await.unwrap();
        assert_eq!(event.workflow_name, "deploy");
        assert_eq!(event.payload, json!({"key": "val"}));
    }

    #[tokio::test]
    async fn sink_errors_when_receiver_dropped() {
        let (sink, rx) = TriggerSink::channel(1);
        drop(rx);

        let result = sink
            .send(TriggerEvent {
                workflow_name: "x".to_string(),
                payload: json!(null),
                trigger_kind: TriggerKind::Manual,
            })
            .await;
        assert!(result.is_err());
    }

    #[test]
    fn trigger_event_clone() {
        let event = TriggerEvent {
            workflow_name: "w".to_string(),
            payload: json!(42),
            trigger_kind: TriggerKind::Api,
        };
        let cloned = event.clone();
        assert_eq!(cloned.workflow_name, "w");
    }
}
