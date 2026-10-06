//! Wake-up of runs paused in `Sleeping`.
//!
//! A run sleeps after a `ctx.delay` step, or while `ctx.wait_for_signal` waits
//! for a signal. Its wake-up time (`scheduled_at`) lives on the run row, so it
//! survives an API or worker restart. The [`Waker`] periodically hands every due
//! run to the engine's [`RunWaker`], which moves it back to `Pending` and, when
//! the API has no worker, resumes it in-process.
//!
//! A run wakes exactly once even when several API instances run this loop.

use std::sync::Arc;
use std::time::Duration;

use ironflow_engine::engine::Engine;
use ironflow_engine::wake::RunWaker;
use tokio::select;
use tokio::time::interval;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

/// How often due sleeping runs are collected.
///
/// Also the worst-case lag between a delay elapsing (or a signal deadline
/// passing) and the run resuming.
pub const DEFAULT_WAKER_INTERVAL: Duration = Duration::from_secs(10);

/// How many runs a single tick wakes.
///
/// Bounded so that a burst of due runs (after a long outage) resorbs
/// progressively instead of holding a long transaction on the runs table.
pub const DEFAULT_WAKER_BATCH_SIZE: u32 = 50;

/// Periodic task that wakes `Sleeping` runs whose `scheduled_at` has passed.
///
/// # Examples
///
/// ```no_run
/// use std::sync::Arc;
/// use std::time::Duration;
/// use ironflow_api::waker::Waker;
/// use ironflow_core::providers::claude::ClaudeCodeProvider;
/// use ironflow_engine::engine::Engine;
/// use ironflow_store::memory::InMemoryStore;
/// use ironflow_store::store::Store;
/// use tokio_util::sync::CancellationToken;
///
/// # async fn example() {
/// let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
/// let engine = Arc::new(Engine::new(store, Arc::new(ClaudeCodeProvider::new())));
///
/// let waker = Waker::new(engine).interval(Duration::from_secs(5));
/// tokio::spawn(waker.run(CancellationToken::new()));
/// # }
/// ```
pub struct Waker {
    waker: RunWaker,
    interval: Duration,
}

impl Waker {
    /// Create a waker with the default interval and batch size.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::sync::Arc;
    /// use ironflow_api::waker::Waker;
    /// use ironflow_core::providers::claude::ClaudeCodeProvider;
    /// use ironflow_engine::engine::Engine;
    /// use ironflow_store::memory::InMemoryStore;
    /// use ironflow_store::store::Store;
    ///
    /// let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
    /// let engine = Arc::new(Engine::new(store, Arc::new(ClaudeCodeProvider::new())));
    /// let waker = Waker::new(engine);
    /// ```
    pub fn new(engine: Arc<Engine>) -> Self {
        Self {
            waker: RunWaker::new(engine).batch_size(DEFAULT_WAKER_BATCH_SIZE),
            interval: DEFAULT_WAKER_INTERVAL,
        }
    }

    /// Set how often due runs are collected.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::sync::Arc;
    /// use std::time::Duration;
    /// use ironflow_api::waker::Waker;
    /// use ironflow_core::providers::claude::ClaudeCodeProvider;
    /// use ironflow_engine::engine::Engine;
    /// use ironflow_store::memory::InMemoryStore;
    /// use ironflow_store::store::Store;
    ///
    /// let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
    /// let engine = Arc::new(Engine::new(store, Arc::new(ClaudeCodeProvider::new())));
    /// let waker = Waker::new(engine).interval(Duration::from_secs(5));
    /// ```
    pub fn interval(mut self, interval: Duration) -> Self {
        self.interval = interval;
        self
    }

    /// Set how many runs a single tick wakes.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::sync::Arc;
    /// use ironflow_api::waker::Waker;
    /// use ironflow_core::providers::claude::ClaudeCodeProvider;
    /// use ironflow_engine::engine::Engine;
    /// use ironflow_store::memory::InMemoryStore;
    /// use ironflow_store::store::Store;
    ///
    /// let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
    /// let engine = Arc::new(Engine::new(store, Arc::new(ClaudeCodeProvider::new())));
    /// let waker = Waker::new(engine).batch_size(10);
    /// ```
    pub fn batch_size(self, batch_size: u32) -> Self {
        Self {
            waker: self.waker.batch_size(batch_size),
            interval: self.interval,
        }
    }

    /// Run the wake-up loop until `shutdown` is cancelled.
    ///
    /// Store errors are logged and the loop keeps going: a transient database
    /// failure must not leave runs asleep forever.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_api::waker::Waker;
    /// use tokio_util::sync::CancellationToken;
    ///
    /// # async fn example(waker: Waker) {
    /// let shutdown = CancellationToken::new();
    /// tokio::spawn(waker.run(shutdown.clone()));
    /// shutdown.cancel();
    /// # }
    /// ```
    pub async fn run(self, shutdown: CancellationToken) {
        let mut ticker = interval(self.interval);
        // The first tick fires immediately; skip it so startup is not a burst.
        ticker.tick().await;

        info!(
            interval_secs = self.interval.as_secs(),
            "sleeping run waker started"
        );

        loop {
            select! {
                _ = shutdown.cancelled() => {
                    info!("sleeping run waker stopped");
                    return;
                }
                _ = ticker.tick() => {
                    self.tick().await;
                }
            }
        }
    }

    /// Wake one batch of due runs.
    ///
    /// Exposed for tests and for callers that drive the schedule themselves.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_api::waker::Waker;
    ///
    /// # async fn example(waker: Waker) {
    /// waker.tick().await;
    /// # }
    /// ```
    pub async fn tick(&self) {
        let runs = match self.waker.tick().await {
            Ok(runs) => runs,
            Err(err) => {
                error!(error = %err, "failed to wake due sleeping runs");
                return;
            }
        };

        for run in &runs {
            info!(
                run_id = %run.id,
                workflow = %run.workflow_name,
                "sleeping run requeued"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use chrono::{DateTime, TimeDelta, Utc};
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::engine::ExecutionMode;
    use ironflow_store::entities::{NewRun, RunStatus, RunUpdate, TriggerKind};
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::store::{RunStore, Store};
    use serde_json::json;
    use tokio::time::timeout;
    use uuid::Uuid;

    use super::*;

    /// Build a waker (workers mode: woken runs stay `Pending`) over a store
    /// holding one run sleeping until `scheduled_at`.
    async fn sleeping_run(scheduled_at: DateTime<Utc>) -> (Arc<InMemoryStore>, Waker, Uuid) {
        let store = Arc::new(InMemoryStore::new());
        let store_dyn: Arc<dyn Store> = store.clone();
        let engine = Arc::new(
            Engine::new(store_dyn, Arc::new(ClaudeCodeProvider::new()))
                .with_execution_mode(ExecutionMode::Workers),
        );

        let run = store
            .create_run(NewRun {
                created_by: None,
                workflow_name: "deploy".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 0,
                handler_version: None,
                labels: HashMap::new(),
                scheduled_at: None,
                idempotency_key: None,
                concurrency_key: None,
                priority: 0,
                concurrency_limits: Vec::new(),
                max_cost_usd: None,
                worker_tags: Vec::new(),
            })
            .await
            .expect("create run")
            .into_run();
        store
            .update_run_status(run.id, RunStatus::Running)
            .await
            .expect("to running");
        store
            .update_run(
                run.id,
                RunUpdate {
                    status: Some(RunStatus::Sleeping),
                    scheduled_at: Some(scheduled_at),
                    ..RunUpdate::default()
                },
            )
            .await
            .expect("to sleeping");

        (store, Waker::new(engine), run.id)
    }

    #[tokio::test]
    async fn waker_tick_requeues_due_sleeping_run() {
        let (store, waker, run_id) = sleeping_run(Utc::now() - TimeDelta::seconds(1)).await;

        waker.tick().await;

        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::Pending);
        assert!(run.scheduled_at.is_none());
    }

    #[tokio::test]
    async fn waker_tick_leaves_future_sleeping_run() {
        let (store, waker, run_id) = sleeping_run(Utc::now() + TimeDelta::hours(1)).await;

        waker.tick().await;

        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::Sleeping);
        assert!(run.scheduled_at.is_some());
    }

    #[tokio::test]
    async fn waker_run_stops_on_shutdown() {
        let (_store, waker, _run_id) = sleeping_run(Utc::now()).await;
        let shutdown = CancellationToken::new();
        shutdown.cancel();

        // Returns instead of looping forever.
        timeout(Duration::from_secs(5), waker.run(shutdown))
            .await
            .expect("waker stopped");
    }
}
