//! Polling trigger for external data sources.
//!
//! [`PollingTrigger`] periodically executes a [`PollingProbe`] and emits a
//! [`TriggerEvent`] when the probe detects new data.
//! Optional deduplication prevents re-triggering when the probe result has
//! not changed since the last poll.
//!
//! # Built-in probes
//!
//! - [`HttpProbe`](http::HttpProbe) -- polls an HTTP endpoint, triggers when the
//!   response body changes (behind the `trigger-polling-http` feature flag).
//! - [`SqlProbe`](sql::SqlProbe) -- executes a SQL query, triggers when the
//!   result set is non-empty (behind the `trigger-polling-sql` feature flag).
//!
//! # Custom probes
//!
//! Implement [`PollingProbe`] to add your own data source:
//!
//! ```no_run
//! use ironflow_runtime::trigger::polling::{PollingProbe, ProbeFuture, ProbeResult, ProbeError};
//!
//! struct MyProbe;
//!
//! impl PollingProbe for MyProbe {
//!     fn name(&self) -> &str { "my-probe" }
//!
//!     fn poll(&self) -> ProbeFuture<'_> {
//!         Box::pin(async {
//!             Ok(Some(ProbeResult::new(serde_json::json!({"rows": 42}))))
//!         })
//!     }
//! }
//! ```
//!
//! # Examples
//!
//! ```no_run
//! use std::time::Duration;
//! use ironflow_runtime::trigger::polling::{
//!     PollingTrigger, PollingTriggerConfig, PollingProbe, ProbeFuture, ProbeResult,
//!     ProbeError,
//! };
//!
//! struct StubProbe;
//! impl PollingProbe for StubProbe {
//!     fn name(&self) -> &str { "stub" }
//!     fn poll(&self) -> ProbeFuture<'_> {
//!         Box::pin(async { Ok(Some(ProbeResult::new(serde_json::json!({"ok": true})))) })
//!     }
//! }
//!
//! let trigger = PollingTrigger::new(PollingTriggerConfig {
//!     interval: Duration::from_secs(30),
//!     probe: Box::new(StubProbe),
//!     workflow_name: "ingest".to_string(),
//!     dedup: true,
//! });
//! ```

#[cfg(feature = "trigger-polling-http")]
pub mod http;
#[cfg(feature = "trigger-polling-sql")]
pub mod sql;

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::time::interval;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use ironflow_store::entities::TriggerKind;

use super::{Trigger, TriggerEvent, TriggerFuture, TriggerSink};

/// Future returned by [`PollingProbe::poll`].
///
/// Returns `Ok(Some(result))` when data is found, `Ok(None)` when the
/// source has nothing to report, or `Err` on failure.
pub type ProbeFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Option<ProbeResult>, ProbeError>> + Send + 'a>>;

/// A probe that checks an external data source for new data.
///
/// Implementations are called periodically by [`PollingTrigger`]. The probe
/// returns `Some(ProbeResult)` when data is found, or `None` when the
/// source has nothing to report (e.g. a SQL query returned zero rows).
///
/// # Examples
///
/// ```no_run
/// use ironflow_runtime::trigger::polling::{PollingProbe, ProbeFuture, ProbeResult, ProbeError};
///
/// struct AlwaysNewProbe;
///
/// impl PollingProbe for AlwaysNewProbe {
///     fn name(&self) -> &str { "always-new" }
///
///     fn poll(&self) -> ProbeFuture<'_> {
///         Box::pin(async {
///             Ok(Some(ProbeResult::new(serde_json::json!({"data": "fresh"}))))
///         })
///     }
/// }
/// ```
pub trait PollingProbe: Send + Sync {
    /// Human-readable name for logging and metrics.
    fn name(&self) -> &str;

    /// Execute the probe and return the result.
    ///
    /// # Errors
    ///
    /// Returns [`ProbeError`] if the probe cannot reach the data source
    /// or encounters an unrecoverable error.
    fn poll(&self) -> ProbeFuture<'_>;
}

/// The result of a successful probe execution.
///
/// Contains the data payload and a SHA-256 content hash used for
/// deduplication.
///
/// # Examples
///
/// ```
/// use ironflow_runtime::trigger::polling::ProbeResult;
/// use serde_json::json;
///
/// let result = ProbeResult::new(json!({"rows": 5}));
/// assert!(!result.content_hash().is_empty());
/// assert_eq!(result.data()["rows"], 5);
/// ```
#[derive(Debug, Clone)]
pub struct ProbeResult {
    data: Value,
    content_hash: String,
}

impl ProbeResult {
    /// Create a new probe result from a JSON payload.
    ///
    /// The content hash is computed as SHA-256 of the canonical JSON
    /// representation.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_runtime::trigger::polling::ProbeResult;
    /// use serde_json::json;
    ///
    /// let r1 = ProbeResult::new(json!({"a": 1}));
    /// let r2 = ProbeResult::new(json!({"a": 1}));
    /// assert_eq!(r1.content_hash(), r2.content_hash());
    ///
    /// let r3 = ProbeResult::new(json!({"a": 2}));
    /// assert_ne!(r1.content_hash(), r3.content_hash());
    /// ```
    pub fn new(data: Value) -> Self {
        let json_bytes = serde_json::to_vec(&data).unwrap_or_default();
        let hash = Sha256::digest(&json_bytes);
        let content_hash = hex::encode(hash);
        Self { data, content_hash }
    }

    /// Create a new probe result with a pre-computed content hash.
    ///
    /// Use this when the hash source differs from the JSON payload
    /// (e.g. the raw HTTP response body before parsing).
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_runtime::trigger::polling::ProbeResult;
    /// use serde_json::json;
    ///
    /// let result = ProbeResult::with_hash(json!({"body": "..."}), "abc123".to_string());
    /// assert_eq!(result.content_hash(), "abc123");
    /// ```
    pub fn with_hash(data: Value, content_hash: String) -> Self {
        Self { data, content_hash }
    }

    /// The data payload.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_runtime::trigger::polling::ProbeResult;
    /// use serde_json::json;
    ///
    /// let result = ProbeResult::new(json!({"count": 3}));
    /// assert_eq!(result.data()["count"], 3);
    /// ```
    pub fn data(&self) -> &Value {
        &self.data
    }

    /// The SHA-256 content hash.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_runtime::trigger::polling::ProbeResult;
    /// use serde_json::json;
    ///
    /// let result = ProbeResult::new(json!({"key": "val"}));
    /// assert_eq!(result.content_hash().len(), 64);
    /// ```
    pub fn content_hash(&self) -> &str {
        &self.content_hash
    }
}

/// Error type for probe operations.
///
/// # Examples
///
/// ```
/// use ironflow_runtime::trigger::polling::ProbeError;
///
/// let err = ProbeError::Failed("connection refused".to_string());
/// assert!(err.to_string().contains("connection refused"));
/// ```
#[derive(Debug, thiserror::Error)]
pub enum ProbeError {
    /// The probe encountered an unrecoverable error.
    #[error("probe failed: {0}")]
    Failed(String),
}

/// Configuration for a [`PollingTrigger`].
///
/// # Examples
///
/// ```no_run
/// use std::time::Duration;
/// use ironflow_runtime::trigger::polling::{
///     PollingTriggerConfig, PollingProbe, ProbeFuture, ProbeResult,
/// };
///
/// struct Stub;
/// impl PollingProbe for Stub {
///     fn name(&self) -> &str { "stub" }
///     fn poll(&self) -> ProbeFuture<'_> {
///         Box::pin(async { Ok(Some(ProbeResult::new(serde_json::json!(null)))) })
///     }
/// }
///
/// let config = PollingTriggerConfig {
///     interval: Duration::from_secs(60),
///     probe: Box::new(Stub),
///     workflow_name: "my-workflow".to_string(),
///     dedup: true,
/// };
/// ```
pub struct PollingTriggerConfig {
    /// How often to poll.
    pub interval: Duration,
    /// The probe to execute.
    pub probe: Box<dyn PollingProbe>,
    /// The workflow to trigger when the probe detects new data.
    pub workflow_name: String,
    /// When `true`, skip triggering if the probe result hash matches the
    /// previous one.
    pub dedup: bool,
}

/// Metric name constants for polling triggers.
#[cfg(feature = "prometheus")]
mod metric_names {
    /// Counter incremented on every poll attempt.
    pub const POLLING_TRIGGER_TOTAL: &str = "ironflow_polling_trigger_total";
}

/// A trigger that periodically polls an external source.
///
/// See the [module-level documentation](self) for usage examples.
///
/// # Examples
///
/// ```no_run
/// use std::time::Duration;
/// use ironflow_runtime::trigger::polling::{
///     PollingTrigger, PollingTriggerConfig, PollingProbe, ProbeFuture, ProbeResult,
/// };
///
/// struct Stub;
/// impl PollingProbe for Stub {
///     fn name(&self) -> &str { "stub" }
///     fn poll(&self) -> ProbeFuture<'_> {
///         Box::pin(async { Ok(Some(ProbeResult::new(serde_json::json!(null)))) })
///     }
/// }
///
/// let trigger = PollingTrigger::new(PollingTriggerConfig {
///     interval: Duration::from_secs(30),
///     probe: Box::new(Stub),
///     workflow_name: "ingest".to_string(),
///     dedup: false,
/// });
/// ```
pub struct PollingTrigger {
    config: PollingTriggerConfig,
}

impl PollingTrigger {
    /// Create a new polling trigger.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::time::Duration;
    /// use ironflow_runtime::trigger::polling::{
    ///     PollingTrigger, PollingTriggerConfig, PollingProbe, ProbeFuture, ProbeResult,
    /// };
    ///
    /// struct Stub;
    /// impl PollingProbe for Stub {
    ///     fn name(&self) -> &str { "stub" }
    ///     fn poll(&self) -> ProbeFuture<'_> {
    ///         Box::pin(async { Ok(Some(ProbeResult::new(serde_json::json!(null)))) })
    ///     }
    /// }
    ///
    /// let trigger = PollingTrigger::new(PollingTriggerConfig {
    ///     interval: Duration::from_secs(10),
    ///     probe: Box::new(Stub),
    ///     workflow_name: "check".to_string(),
    ///     dedup: true,
    /// });
    /// ```
    pub fn new(config: PollingTriggerConfig) -> Self {
        Self { config }
    }

    /// Record a Prometheus metric for a poll outcome.
    #[cfg(feature = "prometheus")]
    fn record_metric(probe_name: &str, outcome: &str) {
        use metrics::counter;

        counter!(
            metric_names::POLLING_TRIGGER_TOTAL,
            "probe" => probe_name.to_string(),
            "outcome" => outcome.to_string()
        )
        .increment(1);
    }
}

impl Trigger for PollingTrigger {
    fn name(&self) -> &str {
        "polling-trigger"
    }

    fn start<'a>(&'a self, sink: TriggerSink, token: &'a CancellationToken) -> TriggerFuture<'a> {
        Box::pin(async move {
            let mut ticker = interval(self.config.interval);
            let mut last_hash: Option<String> = None;
            let probe_name = self.config.probe.name().to_string();

            info!(
                probe = %probe_name,
                interval_secs = self.config.interval.as_secs(),
                workflow = %self.config.workflow_name,
                dedup = self.config.dedup,
                "polling trigger started"
            );

            loop {
                tokio::select! {
                    _ = token.cancelled() => {
                        info!(probe = %probe_name, "polling trigger shutting down");
                        return Ok(());
                    }
                    _ = ticker.tick() => {
                        match self.config.probe.poll().await {
                            Ok(Some(result)) => {
                                let should_trigger = if self.config.dedup {
                                    match &last_hash {
                                        Some(prev) => prev != result.content_hash(),
                                        None => true,
                                    }
                                } else {
                                    true
                                };

                                if should_trigger {
                                    let event = TriggerEvent {
                                        workflow_name: self.config.workflow_name.clone(),
                                        payload: result.data().clone(),
                                        trigger_kind: TriggerKind::Polling {
                                            probe: probe_name.clone(),
                                        },
                                    };

                                    if let Err(e) = sink.send(event).await {
                                        warn!(
                                            probe = %probe_name,
                                            error = %e,
                                            "failed to emit polling trigger event"
                                        );
                                        return Err(e);
                                    }

                                    info!(
                                        probe = %probe_name,
                                        workflow = %self.config.workflow_name,
                                        "polling trigger fired"
                                    );
                                    #[cfg(feature = "prometheus")]
                                    Self::record_metric(&probe_name, "triggered");

                                    last_hash = Some(result.content_hash().to_string());
                                } else {
                                    info!(
                                        probe = %probe_name,
                                        "poll result unchanged, skipping"
                                    );
                                    #[cfg(feature = "prometheus")]
                                    Self::record_metric(&probe_name, "unchanged");
                                }
                            }
                            Ok(None) => {
                                info!(
                                    probe = %probe_name,
                                    "probe returned no data, skipping"
                                );
                                #[cfg(feature = "prometheus")]
                                Self::record_metric(&probe_name, "empty");
                            }
                            Err(e) => {
                                warn!(
                                    probe = %probe_name,
                                    error = %e,
                                    "probe error, skipping this poll"
                                );
                                #[cfg(feature = "prometheus")]
                                Self::record_metric(&probe_name, "error");
                            }
                        }
                    }
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

    use serde_json::json;
    use tokio::time::timeout;
    use tokio_util::sync::CancellationToken;

    use super::*;

    struct FixedProbe {
        data: Value,
    }

    impl FixedProbe {
        fn new(data: Value) -> Self {
            Self { data }
        }
    }

    impl PollingProbe for FixedProbe {
        fn name(&self) -> &str {
            "fixed"
        }

        fn poll(&self) -> ProbeFuture<'_> {
            let data = self.data.clone();
            Box::pin(async move { Ok(Some(ProbeResult::new(data))) })
        }
    }

    struct CountingProbe {
        counter: Arc<AtomicU32>,
    }

    impl CountingProbe {
        fn new(counter: Arc<AtomicU32>) -> Self {
            Self { counter }
        }
    }

    impl PollingProbe for CountingProbe {
        fn name(&self) -> &str {
            "counting"
        }

        fn poll(&self) -> ProbeFuture<'_> {
            let n = self.counter.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move { Ok(Some(ProbeResult::new(json!({"poll": n})))) })
        }
    }

    struct EmptyProbe;

    impl PollingProbe for EmptyProbe {
        fn name(&self) -> &str {
            "empty"
        }

        fn poll(&self) -> ProbeFuture<'_> {
            Box::pin(async { Ok(None) })
        }
    }

    struct ErrorProbe;

    impl PollingProbe for ErrorProbe {
        fn name(&self) -> &str {
            "error"
        }

        fn poll(&self) -> ProbeFuture<'_> {
            Box::pin(async { Err(ProbeError::Failed("connection refused".to_string())) })
        }
    }

    fn make_trigger(probe: Box<dyn PollingProbe>, dedup: bool) -> PollingTrigger {
        PollingTrigger::new(PollingTriggerConfig {
            interval: Duration::from_millis(50),
            probe,
            workflow_name: "test-workflow".to_string(),
            dedup,
        })
    }

    #[tokio::test]
    async fn polling_triggers_on_first_poll() {
        let trigger = make_trigger(Box::new(FixedProbe::new(json!({"key": "val"}))), true);
        let (sink, mut rx) = TriggerSink::channel(16);
        let token = CancellationToken::new();
        let token_clone = token.clone();

        let handle = tokio::spawn(async move { trigger.start(sink, &token_clone).await });

        let event = timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("timed out")
            .expect("channel closed");

        assert_eq!(event.workflow_name, "test-workflow");
        assert_eq!(event.payload["key"], "val");
        assert!(matches!(
            event.trigger_kind,
            TriggerKind::Polling { ref probe } if probe == "fixed"
        ));

        token.cancel();
        let _ = handle.await;
    }

    #[tokio::test]
    async fn polling_dedup_skips_unchanged() {
        let trigger = make_trigger(Box::new(FixedProbe::new(json!({"static": true}))), true);
        let (sink, mut rx) = TriggerSink::channel(16);
        let token = CancellationToken::new();
        let token_clone = token.clone();

        let handle = tokio::spawn(async move { trigger.start(sink, &token_clone).await });

        // First poll triggers
        let _event = timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("timed out")
            .expect("channel closed");

        // Wait for a couple more poll cycles
        tokio::time::sleep(Duration::from_millis(150)).await;

        // No second event should arrive
        assert!(rx.try_recv().is_err());

        token.cancel();
        let _ = handle.await;
    }

    #[tokio::test]
    async fn polling_no_dedup_fires_every_time() {
        let trigger = make_trigger(Box::new(FixedProbe::new(json!({"static": true}))), false);
        let (sink, mut rx) = TriggerSink::channel(16);
        let token = CancellationToken::new();
        let token_clone = token.clone();

        let handle = tokio::spawn(async move { trigger.start(sink, &token_clone).await });

        // Should get at least 2 events even with identical data
        let _e1 = timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("timed out")
            .expect("channel closed");

        let _e2 = timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("timed out")
            .expect("channel closed");

        token.cancel();
        let _ = handle.await;
    }

    #[tokio::test]
    async fn polling_error_does_not_trigger() {
        let trigger = make_trigger(Box::new(ErrorProbe), false);
        let (sink, mut rx) = TriggerSink::channel(16);
        let token = CancellationToken::new();
        let token_clone = token.clone();

        let handle = tokio::spawn(async move { trigger.start(sink, &token_clone).await });

        // Wait for a few poll cycles
        tokio::time::sleep(Duration::from_millis(200)).await;

        // No event should have been emitted
        assert!(rx.try_recv().is_err());

        token.cancel();
        let _ = handle.await;
    }

    #[tokio::test]
    async fn polling_graceful_shutdown() {
        let trigger = make_trigger(Box::new(FixedProbe::new(json!(null))), false);
        let (sink, _rx) = TriggerSink::channel(16);
        let token = CancellationToken::new();
        let token_clone = token.clone();

        let handle = tokio::spawn(async move { trigger.start(sink, &token_clone).await });

        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!handle.is_finished());

        token.cancel();
        let result = timeout(Duration::from_secs(2), handle)
            .await
            .expect("timed out")
            .expect("task panicked");
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn polling_dedup_fires_on_change() {
        let counter = Arc::new(AtomicU32::new(0));
        let trigger = make_trigger(Box::new(CountingProbe::new(counter)), true);
        let (sink, mut rx) = TriggerSink::channel(16);
        let token = CancellationToken::new();
        let token_clone = token.clone();

        let handle = tokio::spawn(async move { trigger.start(sink, &token_clone).await });

        // Every poll returns different data, so dedup should NOT block
        let _e1 = timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("timed out")
            .expect("channel closed");

        let _e2 = timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("timed out")
            .expect("channel closed");

        token.cancel();
        let _ = handle.await;
    }

    #[tokio::test]
    async fn polling_empty_probe_does_not_trigger() {
        let trigger = make_trigger(Box::new(EmptyProbe), false);
        let (sink, mut rx) = TriggerSink::channel(16);
        let token = CancellationToken::new();
        let token_clone = token.clone();

        let handle = tokio::spawn(async move { trigger.start(sink, &token_clone).await });

        tokio::time::sleep(Duration::from_millis(200)).await;

        assert!(rx.try_recv().is_err());

        token.cancel();
        let _ = handle.await;
    }

    #[test]
    fn probe_result_hash_deterministic() {
        let r1 = ProbeResult::new(json!({"a": 1, "b": 2}));
        let r2 = ProbeResult::new(json!({"a": 1, "b": 2}));
        assert_eq!(r1.content_hash(), r2.content_hash());
    }

    #[test]
    fn probe_result_hash_differs_for_different_data() {
        let r1 = ProbeResult::new(json!({"a": 1}));
        let r2 = ProbeResult::new(json!({"a": 2}));
        assert_ne!(r1.content_hash(), r2.content_hash());
    }

    #[test]
    fn probe_result_with_custom_hash() {
        let r = ProbeResult::with_hash(json!(null), "custom-hash".to_string());
        assert_eq!(r.content_hash(), "custom-hash");
    }

    #[test]
    fn probe_error_display() {
        let err = ProbeError::Failed("timeout".to_string());
        assert_eq!(err.to_string(), "probe failed: timeout");
    }

    #[cfg(feature = "prometheus")]
    #[tokio::test]
    async fn polling_metrics_incremented() {
        use metrics_exporter_prometheus::PrometheusBuilder;

        let recorder = PrometheusBuilder::new().build_recorder();
        let prom_handle = recorder.handle();
        metrics::set_global_recorder(recorder).ok();

        let trigger = make_trigger(Box::new(FixedProbe::new(json!({"m": 1}))), false);
        let (sink, mut rx) = TriggerSink::channel(16);
        let token = CancellationToken::new();
        let token_clone = token.clone();

        let handle = tokio::spawn(async move { trigger.start(sink, &token_clone).await });

        let _ = timeout(Duration::from_secs(2), rx.recv()).await;

        token.cancel();
        let _ = handle.await;

        let output = prom_handle.render();
        assert!(
            output.contains("ironflow_polling_trigger_total"),
            "expected polling metric in: {output}"
        );
    }
}
