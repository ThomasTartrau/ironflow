//! Recovery of runs abandoned by a dead worker.
//!
//! A worker attaches a lease to every run it picks up and refreshes it while it
//! executes. When the worker dies (OOM, evicted pod, hard shutdown), the lease
//! stops being refreshed and the run would otherwise stay `Running` forever.
//! The [`Reaper`] periodically requeues those runs.
//!
//! Runs executed in-process by the API server (inline execution, resume after
//! approval) never hold a lease and are never touched by the reaper.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use ironflow_engine::engine::Engine;
use ironflow_engine::notify::Event;
use ironflow_store::entities::{ReapedRun, RunStatus};
use ironflow_store::store::{LEASE_EXPIRED_ERROR, Store};
use tokio::time::interval;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

#[cfg(feature = "prometheus")]
use ironflow_core::metric_names::RUNS_REAPED_TOTAL;
#[cfg(feature = "prometheus")]
use metrics::counter;

/// How often expired leases are collected.
pub const DEFAULT_REAPER_INTERVAL: Duration = Duration::from_secs(60);

/// How many runs a single tick recovers.
///
/// Bounded so that a mass failure (a whole worker fleet dying at once) resorbs
/// progressively instead of holding a long transaction on the runs table.
pub const DEFAULT_REAPER_BATCH_SIZE: u32 = 100;

/// Periodic task that requeues runs whose worker lease expired.
///
/// # Examples
///
/// ```no_run
/// use std::sync::Arc;
/// use std::time::Duration;
/// use ironflow_api::reaper::Reaper;
/// use ironflow_core::providers::claude::ClaudeCodeProvider;
/// use ironflow_engine::engine::Engine;
/// use ironflow_store::memory::InMemoryStore;
/// use ironflow_store::store::Store;
/// use tokio_util::sync::CancellationToken;
///
/// # async fn example() {
/// let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
/// let engine = Arc::new(Engine::new(store.clone(), Arc::new(ClaudeCodeProvider::new())));
///
/// let reaper = Reaper::new(store, engine).interval(Duration::from_secs(30));
/// tokio::spawn(reaper.run(CancellationToken::new()));
/// # }
/// ```
pub struct Reaper {
    store: Arc<dyn Store>,
    engine: Arc<Engine>,
    interval: Duration,
    batch_size: u32,
}

impl Reaper {
    /// Create a reaper with the default interval and batch size.
    pub fn new(store: Arc<dyn Store>, engine: Arc<Engine>) -> Self {
        Self {
            store,
            engine,
            interval: DEFAULT_REAPER_INTERVAL,
            batch_size: DEFAULT_REAPER_BATCH_SIZE,
        }
    }

    /// Set how often expired leases are collected.
    ///
    /// Keep it well below the worker lease TTL, otherwise recovery takes longer
    /// than the TTL suggests.
    pub fn interval(mut self, interval: Duration) -> Self {
        self.interval = interval;
        self
    }

    /// Set how many runs a single tick recovers.
    pub fn batch_size(mut self, batch_size: u32) -> Self {
        self.batch_size = batch_size;
        self
    }

    /// Run the recovery loop until `shutdown` is cancelled.
    ///
    /// Store errors are logged and the loop keeps going: a transient database
    /// failure must not silently stop recovery.
    pub async fn run(self, shutdown: CancellationToken) {
        let mut ticker = interval(self.interval);
        // The first tick fires immediately; skip it so startup is not a burst.
        ticker.tick().await;

        info!(
            interval_secs = self.interval.as_secs(),
            batch_size = self.batch_size,
            "reaper started"
        );

        loop {
            tokio::select! {
                _ = shutdown.cancelled() => {
                    info!("reaper stopped");
                    return;
                }
                _ = ticker.tick() => {
                    self.tick().await;
                }
            }
        }
    }

    /// Recover one batch of expired leases.
    ///
    /// Exposed for tests and for callers that drive the schedule themselves.
    pub async fn tick(&self) {
        let reaped = match self.store.reap_expired_leases(self.batch_size).await {
            Ok(reaped) => reaped,
            Err(err) => {
                error!(error = %err, "failed to collect expired leases");
                return;
            }
        };

        if reaped.is_empty() {
            return;
        }

        warn!(
            count = reaped.len(),
            batch_size = self.batch_size,
            "recovered runs with an expired worker lease"
        );

        for entry in &reaped {
            self.finish_recovery(entry).await;
        }
    }

    /// Apply the side effects of a recovery: clean up steps, publish the event.
    async fn finish_recovery(&self, entry: &ReapedRun) {
        let run = &entry.run;

        warn!(
            run_id = %run.id,
            workflow = %run.workflow_name,
            worker_id = run.worker_id.as_deref().unwrap_or("unknown"),
            retry_count = run.retry_count,
            to = %entry.to,
            "worker lease expired"
        );

        if let Err(err) = self
            .engine
            .fail_orphaned_steps(run.id, LEASE_EXPIRED_ERROR)
            .await
        {
            error!(run_id = %run.id, error = %err, "failed to clean up orphaned steps");
        }

        #[cfg(feature = "prometheus")]
        {
            let outcome = if entry.to == RunStatus::Failed {
                "failed"
            } else {
                "requeued"
            };
            counter!(RUNS_REAPED_TOTAL, "outcome" => outcome).increment(1);
        }

        self.engine
            .event_publisher()
            .publish(Event::RunStatusChanged {
                run_id: run.id,
                workflow_name: run.workflow_name.clone(),
                from: entry.from,
                to: entry.to,
                error: (entry.to == RunStatus::Failed).then(|| LEASE_EXPIRED_ERROR.to_string()),
                cost_usd: run.cost_usd,
                duration_ms: run.duration_ms,
                labels: run.labels.clone(),
                at: Utc::now(),
            });
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::notify::{EventSubscriber, SubscriberFuture};
    use ironflow_store::entities::{
        LeaseRequest, NewRun, NewStep, RunFilter, StepKind, StepStatus, StepUpdate, TriggerKind,
    };
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::store::RunStore;
    use serde_json::json;
    use tokio::task::yield_now;
    use tokio::time::sleep;
    use uuid::Uuid;

    use super::*;

    fn new_run(max_retries: u32) -> NewRun {
        NewRun {
            workflow_name: "test".to_string(),
            trigger: TriggerKind::Manual,
            payload: json!({}),
            max_retries,
            handler_version: None,
            labels: HashMap::new(),
            scheduled_at: None,
            created_by: None,
            idempotency_key: None,
            max_cost_usd: None,
        }
    }

    fn lease(worker_id: &str, ttl: Duration) -> Option<LeaseRequest> {
        Some(LeaseRequest {
            worker_id: worker_id.to_string(),
            ttl,
        })
    }

    fn build(store: Arc<InMemoryStore>) -> (Reaper, Arc<Engine>) {
        let store_dyn: Arc<dyn Store> = store;
        let engine = Arc::new(Engine::new(
            store_dyn.clone(),
            Arc::new(ClaudeCodeProvider::new()),
        ));
        (Reaper::new(store_dyn, engine.clone()), engine)
    }

    /// Collects the events published during a test.
    #[derive(Default)]
    struct EventRecorder {
        events: Mutex<Vec<Event>>,
    }

    impl EventRecorder {
        fn events(&self) -> Vec<Event> {
            self.events.lock().expect("recorder lock").clone()
        }
    }

    struct RecorderHandle(Arc<EventRecorder>);

    impl EventSubscriber for RecorderHandle {
        fn name(&self) -> &str {
            "test-recorder"
        }

        fn handle<'a>(&'a self, event: &'a Event) -> SubscriberFuture<'a> {
            Box::pin(async move {
                self.0
                    .events
                    .lock()
                    .expect("recorder lock")
                    .push(event.clone());
            })
        }
    }

    /// Build a reaper whose engine records every published event.
    fn build_recording(store: Arc<InMemoryStore>) -> (Reaper, Arc<EventRecorder>) {
        let store_dyn: Arc<dyn Store> = store;
        let mut engine = Engine::new(store_dyn.clone(), Arc::new(ClaudeCodeProvider::new()));
        let recorder = Arc::new(EventRecorder::default());
        engine.subscribe(RecorderHandle(recorder.clone()), Event::ALL);
        (Reaper::new(store_dyn, Arc::new(engine)), recorder)
    }

    /// Pick a run with a lease that is already expired.
    ///
    /// The short sleep matters: `Utc::now()` has microsecond resolution, so a
    /// sub-microsecond TTL can still read as "not yet expired" in the same tick.
    async fn picked_with_expired_lease(store: &InMemoryStore, max_retries: u32) -> Uuid {
        store.create_run(new_run(max_retries)).await.unwrap();
        let run = store
            .pick_next_pending(lease("worker-1", Duration::from_nanos(1)))
            .await
            .unwrap()
            .unwrap();
        sleep(Duration::from_millis(2)).await;
        run.id
    }

    #[tokio::test]
    async fn tick_requeues_run_with_expired_lease() {
        let store = Arc::new(InMemoryStore::new());
        let run_id = picked_with_expired_lease(&store, 3).await;
        let (reaper, _engine) = build(store.clone());

        reaper.tick().await;

        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::Pending);
        assert_eq!(run.retry_count, 1);
        assert!(run.worker_id.is_none());
        assert!(run.lease_expires_at.is_none());
    }

    #[tokio::test]
    async fn tick_leaves_valid_lease_alone() {
        let store = Arc::new(InMemoryStore::new());
        store.create_run(new_run(3)).await.unwrap();
        let run = store
            .pick_next_pending(lease("worker-1", Duration::from_secs(90)))
            .await
            .unwrap()
            .unwrap();
        let (reaper, _engine) = build(store.clone());

        reaper.tick().await;

        let after = store.get_run(run.id).await.unwrap().unwrap();
        assert_eq!(after.status.state, RunStatus::Running);
        assert_eq!(after.retry_count, 0);
        assert_eq!(after.worker_id.as_deref(), Some("worker-1"));
    }

    #[tokio::test]
    async fn tick_fails_run_once_retries_are_exhausted() {
        let store = Arc::new(InMemoryStore::new());
        let run_id = picked_with_expired_lease(&store, 0).await;
        let (reaper, _engine) = build(store.clone());

        reaper.tick().await;

        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::Failed);
        assert_eq!(run.error.as_deref(), Some(LEASE_EXPIRED_ERROR));
    }

    #[tokio::test]
    async fn tick_fails_orphaned_steps() {
        let store = Arc::new(InMemoryStore::new());
        let run_id = picked_with_expired_lease(&store, 3).await;
        let step = store
            .create_step(NewStep {
                run_id,
                name: "step-1".to_string(),
                kind: StepKind::Shell,
                position: 0,
                input: None,
            })
            .await
            .unwrap();
        store
            .update_step(
                step.id,
                StepUpdate {
                    status: Some(StepStatus::Running),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let (reaper, _engine) = build(store.clone());

        reaper.tick().await;

        let steps = store.list_steps(run_id).await.unwrap();
        assert_eq!(steps[0].status.state, StepStatus::Failed);
    }

    #[tokio::test]
    async fn tick_publishes_status_change_event() {
        let store = Arc::new(InMemoryStore::new());
        let run_id = picked_with_expired_lease(&store, 3).await;
        let (reaper, recorder) = build_recording(store.clone());

        reaper.tick().await;
        // Subscribers run in spawned tasks; give them a turn to record.
        yield_now().await;

        let status_changes: Vec<_> = recorder
            .events()
            .into_iter()
            .filter_map(|event| match event {
                Event::RunStatusChanged {
                    run_id: id,
                    from,
                    to,
                    error,
                    ..
                } if id == run_id => Some((from, to, error)),
                _ => None,
            })
            .collect();

        assert_eq!(
            status_changes,
            vec![(RunStatus::Running, RunStatus::Pending, None)]
        );
    }

    #[tokio::test]
    async fn tick_publishes_error_when_retries_are_exhausted() {
        let store = Arc::new(InMemoryStore::new());
        let run_id = picked_with_expired_lease(&store, 0).await;
        let (reaper, recorder) = build_recording(store.clone());

        reaper.tick().await;
        yield_now().await;

        let matched = recorder.events().into_iter().any(|event| {
            matches!(
                event,
                Event::RunStatusChanged { run_id: id, to, error: Some(err), .. }
                    if id == run_id && to == RunStatus::Failed && err == LEASE_EXPIRED_ERROR
            )
        });
        assert!(matched, "expected a failed status change with an error");
    }

    #[tokio::test]
    async fn tick_respects_batch_size() {
        let store = Arc::new(InMemoryStore::new());
        for _ in 0..3 {
            picked_with_expired_lease(&store, 3).await;
        }
        let (reaper, _engine) = build(store.clone());
        let reaper = reaper.batch_size(2);

        reaper.tick().await;

        let pending = store
            .list_runs(
                RunFilter {
                    status: Some(RunStatus::Pending),
                    ..Default::default()
                },
                1,
                100,
            )
            .await
            .unwrap();
        assert_eq!(pending.total, 2);
    }

    #[tokio::test]
    async fn run_stops_on_shutdown() {
        let store = Arc::new(InMemoryStore::new());
        let (reaper, _engine) = build(store);
        let shutdown = CancellationToken::new();
        shutdown.cancel();

        // Returns instead of looping forever.
        tokio::time::timeout(Duration::from_secs(5), reaper.run(shutdown))
            .await
            .expect("reaper stopped");
    }
}
