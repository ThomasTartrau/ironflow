//! Worker -- polls the API for pending runs and executes them.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::spawn;
use tokio::sync::{Semaphore, mpsc};
use tokio::time::{sleep, timeout};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
use uuid::Uuid;

#[cfg(feature = "prometheus")]
use ironflow_core::metric_names::{WORKER_ACTIVE, WORKER_LEASES_LOST_TOTAL, WORKER_POLLS_TOTAL};
use ironflow_core::provider::AgentProvider;
use ironflow_engine::engine::Engine;
use ironflow_engine::handler::WorkflowHandler;
use ironflow_engine::log_sender::LogReceiver;
use ironflow_store::entities::{LeaseRequest, RunStatus, RunUpdate};
use ironflow_store::error::StoreError;
use ironflow_store::store::Store;
#[cfg(feature = "prometheus")]
use metrics::{counter, gauge};
#[cfg(feature = "heartbeat")]
use reqwest::Client;

use crate::api_store::ApiRunStore;
use crate::error::WorkerError;
use crate::log_pusher::LogPusher;

const DEFAULT_CONCURRENCY: usize = 2;
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(2);
const DEFAULT_RUN_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const DEFAULT_MAX_CONSECUTIVE_PANICS: u32 = 3;
const DEFAULT_PANIC_COOLDOWN: Duration = Duration::from_secs(5 * 60);
/// Lease duration requested when picking a run: three missed refreshes before
/// the run becomes recoverable by the reaper.
const DEFAULT_LEASE_TTL: Duration = Duration::from_secs(90);
/// How often the lease of a running run is refreshed.
const DEFAULT_LEASE_REFRESH_INTERVAL: Duration = Duration::from_secs(30);
#[cfg(feature = "heartbeat")]
const DEFAULT_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);

/// Builder for configuring and creating a [`Worker`].
///
/// # Examples
///
/// ```no_run
/// use std::sync::Arc;
/// use std::time::Duration;
/// use ironflow_worker::WorkerBuilder;
/// use ironflow_core::providers::claude::ClaudeCodeProvider;
///
/// # async fn example() -> Result<(), ironflow_worker::WorkerError> {
/// let worker = WorkerBuilder::new("http://localhost:3000", "my-token")
///     .provider(Arc::new(ClaudeCodeProvider::new()))
///     .concurrency(4)
///     .poll_interval(Duration::from_secs(2))
///     .run_timeout(Duration::from_secs(600))
///     .max_consecutive_panics(5)
///     .build()?;
///
/// worker.run().await?;
/// # Ok(())
/// # }
/// ```
pub struct WorkerBuilder {
    api_url: String,
    worker_token: String,
    worker_id: String,
    provider: Option<Arc<dyn AgentProvider>>,
    handlers: Vec<Box<dyn WorkflowHandler>>,
    concurrency: usize,
    poll_interval: Duration,
    run_timeout: Duration,
    max_consecutive_panics: u32,
    panic_cooldown: Duration,
    lease_ttl: Duration,
    lease_refresh_interval: Duration,
    #[cfg(feature = "heartbeat")]
    heartbeat_url: Option<String>,
    #[cfg(feature = "heartbeat")]
    heartbeat_interval: Duration,
}

impl WorkerBuilder {
    /// Create a new builder targeting the given API server.
    pub fn new(api_url: &str, worker_token: &str) -> Self {
        Self {
            api_url: api_url.to_string(),
            worker_token: worker_token.to_string(),
            worker_id: format!("worker-{}", Uuid::now_v7()),
            provider: None,
            handlers: Vec::new(),
            concurrency: DEFAULT_CONCURRENCY,
            poll_interval: DEFAULT_POLL_INTERVAL,
            run_timeout: DEFAULT_RUN_TIMEOUT,
            max_consecutive_panics: DEFAULT_MAX_CONSECUTIVE_PANICS,
            panic_cooldown: DEFAULT_PANIC_COOLDOWN,
            lease_ttl: DEFAULT_LEASE_TTL,
            lease_refresh_interval: DEFAULT_LEASE_REFRESH_INTERVAL,
            #[cfg(feature = "heartbeat")]
            heartbeat_url: None,
            #[cfg(feature = "heartbeat")]
            heartbeat_interval: DEFAULT_HEARTBEAT_INTERVAL,
        }
    }

    /// Set the agent provider for AI operations.
    pub fn provider(mut self, provider: Arc<dyn AgentProvider>) -> Self {
        self.provider = Some(provider);
        self
    }

    /// Register a workflow handler.
    pub fn register(mut self, handler: impl WorkflowHandler + 'static) -> Self {
        self.handlers.push(Box::new(handler));
        self
    }

    /// Set the maximum number of concurrent workflow executions.
    pub fn concurrency(mut self, n: usize) -> Self {
        self.concurrency = n;
        self
    }

    /// Set the interval between polls for new runs.
    pub fn poll_interval(mut self, interval: Duration) -> Self {
        self.poll_interval = interval;
        self
    }

    /// Set the maximum execution time per run.
    ///
    /// If a run exceeds this duration, it is cancelled and marked as `Failed`
    /// with a timeout error. Defaults to 30 minutes.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::time::Duration;
    /// use ironflow_worker::WorkerBuilder;
    ///
    /// # fn example() {
    /// let builder = WorkerBuilder::new("http://localhost:3000", "token")
    ///     .run_timeout(Duration::from_secs(600));
    /// # }
    /// ```
    pub fn run_timeout(mut self, timeout: Duration) -> Self {
        self.run_timeout = timeout;
        self
    }

    /// Set the maximum number of consecutive panics per workflow before
    /// the worker stops picking runs for that workflow (poison pill guard).
    ///
    /// When a workflow panics `max_consecutive_panics` times in a row without
    /// a single success, the worker skips it for a cooldown period (see
    /// [`panic_cooldown`](Self::panic_cooldown)). Defaults to 3.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_worker::WorkerBuilder;
    ///
    /// # fn example() {
    /// let builder = WorkerBuilder::new("http://localhost:3000", "token")
    ///     .max_consecutive_panics(5);
    /// # }
    /// ```
    pub fn max_consecutive_panics(mut self, n: u32) -> Self {
        self.max_consecutive_panics = n;
        self
    }

    /// Set the cooldown duration after a workflow is flagged as a poison pill.
    ///
    /// After `max_consecutive_panics` is reached, runs for that workflow are
    /// skipped until this duration elapses. Defaults to 5 minutes.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::time::Duration;
    /// use ironflow_worker::WorkerBuilder;
    ///
    /// # fn example() {
    /// let builder = WorkerBuilder::new("http://localhost:3000", "token")
    ///     .panic_cooldown(Duration::from_secs(600));
    /// # }
    /// ```
    pub fn panic_cooldown(mut self, cooldown: Duration) -> Self {
        self.panic_cooldown = cooldown;
        self
    }

    /// Set the identifier this worker uses to claim runs.
    ///
    /// Defaults to `worker-<uuid>`, regenerated at every start so two processes
    /// never share an identity. Override it only if you have a stable, unique
    /// name per process.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_worker::WorkerBuilder;
    ///
    /// # fn example() {
    /// let builder = WorkerBuilder::new("http://localhost:3000", "token")
    ///     .worker_id("worker-eu-west-1a");
    /// # }
    /// ```
    pub fn worker_id(mut self, worker_id: &str) -> Self {
        self.worker_id = worker_id.to_string();
        self
    }

    /// Set how long a lease stays valid without a refresh.
    ///
    /// Once it expires, the API reaper requeues the run for another worker.
    /// Keep it at a few times [`lease_refresh_interval`](Self::lease_refresh_interval)
    /// so a transient network blip does not hand the run over. Defaults to 90 s.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::time::Duration;
    /// use ironflow_worker::WorkerBuilder;
    ///
    /// # fn example() {
    /// let builder = WorkerBuilder::new("http://localhost:3000", "token")
    ///     .lease_ttl(Duration::from_secs(120));
    /// # }
    /// ```
    pub fn lease_ttl(mut self, ttl: Duration) -> Self {
        self.lease_ttl = ttl;
        self
    }

    /// Set how often the lease of a running run is refreshed.
    ///
    /// Defaults to 30 seconds. Must stay well below
    /// [`lease_ttl`](Self::lease_ttl).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::time::Duration;
    /// use ironflow_worker::WorkerBuilder;
    ///
    /// # fn example() {
    /// let builder = WorkerBuilder::new("http://localhost:3000", "token")
    ///     .lease_refresh_interval(Duration::from_secs(10));
    /// # }
    /// ```
    pub fn lease_refresh_interval(mut self, interval: Duration) -> Self {
        self.lease_refresh_interval = interval;
        self
    }

    /// Set the heartbeat URL (dead man's switch).
    ///
    /// The worker pings this URL at every heartbeat interval with an HTTP
    /// HEAD request. Compatible with BetterStack Heartbeats, Cronitor,
    /// Healthchecks.io, or any dead man's switch service.
    ///
    /// If not set, no heartbeat is emitted even when the feature is enabled.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_worker::WorkerBuilder;
    ///
    /// # fn example() {
    /// let builder = WorkerBuilder::new("http://localhost:3000", "token")
    ///     .heartbeat_url("https://uptime.betterstack.com/api/v1/heartbeat/abc123");
    /// # }
    /// ```
    #[cfg(feature = "heartbeat")]
    pub fn heartbeat_url(mut self, url: &str) -> Self {
        self.heartbeat_url = Some(url.to_string());
        self
    }

    /// Set the heartbeat interval.
    ///
    /// Controls how often the worker pings the [`heartbeat_url`](Self::heartbeat_url).
    /// Defaults to 30 seconds.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::time::Duration;
    /// use ironflow_worker::WorkerBuilder;
    ///
    /// # fn example() {
    /// let builder = WorkerBuilder::new("http://localhost:3000", "token")
    ///     .heartbeat_url("https://uptime.betterstack.com/api/v1/heartbeat/abc123")
    ///     .heartbeat_interval(Duration::from_secs(60));
    /// # }
    /// ```
    #[cfg(feature = "heartbeat")]
    pub fn heartbeat_interval(mut self, interval: Duration) -> Self {
        self.heartbeat_interval = interval;
        self
    }

    /// Build the worker.
    ///
    /// # Errors
    ///
    /// Returns [`WorkerError::Internal`] if no provider has been set.
    /// Returns [`WorkerError::Engine`] if a handler registration fails.
    pub fn build(self) -> Result<Worker, WorkerError> {
        let provider = self
            .provider
            .ok_or_else(|| WorkerError::Internal("WorkerBuilder: provider is required".into()))?;

        let store: Arc<dyn Store> = Arc::new(ApiRunStore::new(&self.api_url, &self.worker_token));

        let mut engine = Engine::new(store, provider);
        for handler in self.handlers {
            engine
                .register_boxed(handler)
                .map_err(WorkerError::Engine)?;
        }

        let (log_sender, log_receiver) = ironflow_engine::log_sender::channel();
        engine.set_log_sender(log_sender);

        #[cfg(feature = "heartbeat")]
        let heartbeat_client = Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("failed to build heartbeat HTTP client");

        Ok(Worker {
            engine: Arc::new(engine),
            api_url: self.api_url,
            worker_token: self.worker_token,
            worker_id: self.worker_id,
            log_receiver: Mutex::new(Some(log_receiver)),
            concurrency: self.concurrency,
            poll_interval: self.poll_interval,
            run_timeout: self.run_timeout,
            max_consecutive_panics: self.max_consecutive_panics,
            panic_cooldown: self.panic_cooldown,
            lease_ttl: self.lease_ttl,
            lease_refresh_interval: self.lease_refresh_interval,
            #[cfg(feature = "heartbeat")]
            heartbeat_url: self.heartbeat_url,
            #[cfg(feature = "heartbeat")]
            heartbeat_interval: self.heartbeat_interval,
            #[cfg(feature = "heartbeat")]
            heartbeat_client,
        })
    }
}

/// Background worker that polls the API and executes workflows.
pub struct Worker {
    engine: Arc<Engine>,
    api_url: String,
    worker_token: String,
    worker_id: String,
    log_receiver: Mutex<Option<LogReceiver>>,
    concurrency: usize,
    poll_interval: Duration,
    run_timeout: Duration,
    max_consecutive_panics: u32,
    panic_cooldown: Duration,
    lease_ttl: Duration,
    lease_refresh_interval: Duration,
    #[cfg(feature = "heartbeat")]
    heartbeat_url: Option<String>,
    #[cfg(feature = "heartbeat")]
    heartbeat_interval: Duration,
    #[cfg(feature = "heartbeat")]
    heartbeat_client: Client,
}

/// Tracks consecutive failures per workflow for poison pill detection.
struct PoisonPillTracker {
    max_consecutive: u32,
    cooldown: Duration,
    /// Maps workflow name to (consecutive panic count, last panic time).
    state: HashMap<String, (u32, Instant)>,
}

impl PoisonPillTracker {
    fn new(max_consecutive: u32, cooldown: Duration) -> Self {
        Self {
            max_consecutive,
            cooldown,
            state: HashMap::new(),
        }
    }

    /// Record a panic for a workflow. Returns `true` if the workflow is now
    /// considered a poison pill.
    fn record_panic(&mut self, workflow: &str) -> bool {
        let entry = self
            .state
            .entry(workflow.to_string())
            .or_insert((0, Instant::now()));
        entry.0 += 1;
        entry.1 = Instant::now();
        entry.0 >= self.max_consecutive
    }

    /// Record a successful execution, resetting the panic counter.
    fn record_success(&mut self, workflow: &str) {
        self.state.remove(workflow);
    }

    /// Returns `true` if the workflow is currently blocked as a poison pill.
    fn is_blocked(&self, workflow: &str) -> bool {
        self.state.get(workflow).is_some_and(|(count, last_panic)| {
            *count >= self.max_consecutive && last_panic.elapsed() < self.cooldown
        })
    }
}

impl Worker {
    /// Run the worker loop until a shutdown signal (SIGTERM/SIGINT) is received.
    ///
    /// On shutdown, the worker stops picking new runs and waits for all
    /// in-flight executions to complete before returning.
    ///
    /// # Errors
    ///
    /// Returns [`WorkerError`] if the polling loop encounters an unrecoverable error.
    pub async fn run(&self) -> Result<(), WorkerError> {
        let semaphore = Arc::new(Semaphore::new(self.concurrency));
        let shutdown = CancellationToken::new();
        let mut idle_streak = 0u32;
        let poison_tracker = Arc::new(Mutex::new(PoisonPillTracker::new(
            self.max_consecutive_panics,
            self.panic_cooldown,
        )));
        let (outcome_tx, mut outcome_rx) = mpsc::unbounded_channel::<RunOutcome>();

        info!(
            concurrency = self.concurrency,
            poll_interval_ms = self.poll_interval.as_millis() as u64,
            run_timeout_secs = self.run_timeout.as_secs(),
            "worker started"
        );

        if let Some(receiver) = self.log_receiver.lock().expect("log_receiver lock").take() {
            let pusher = LogPusher::new(&self.api_url, &self.worker_token);
            spawn(pusher.run(receiver));
            info!("log pusher started");
        }

        // Spawn shutdown signal handler
        let shutdown_clone = shutdown.clone();
        spawn(async move {
            shutdown_signal().await;
            info!("shutdown signal received, draining in-flight runs...");
            shutdown_clone.cancel();
        });

        #[cfg(feature = "heartbeat")]
        if let Some(ref url) = self.heartbeat_url {
            let interval = self.heartbeat_interval;
            let url = url.clone();
            let client = self.heartbeat_client.clone();

            spawn(async move {
                let mut ticker = tokio::time::interval(interval);
                // skip the first immediate tick
                ticker.tick().await;
                loop {
                    ticker.tick().await;
                    match client.head(&url).send().await {
                        Ok(resp) if resp.status().is_success() => {
                            info!(url = %url, "heartbeat sent");
                        }
                        Ok(resp) => {
                            warn!(
                                url = %url,
                                status = %resp.status(),
                                "heartbeat ping returned non-success status"
                            );
                        }
                        Err(err) => {
                            warn!(
                                url = %url,
                                error = %err,
                                "heartbeat ping failed"
                            );
                        }
                    }
                }
            });
        }

        while !shutdown.is_cancelled() {
            // Drain outcome channel to update poison pill tracker
            while let Ok(outcome) = outcome_rx.try_recv() {
                let mut tracker = poison_tracker.lock().expect("poison tracker lock poisoned");
                match outcome {
                    RunOutcome::Success(ref wf) => tracker.record_success(wf),
                    // Losing a lease says nothing about the workflow itself:
                    // it must not count towards the poison pill threshold.
                    RunOutcome::LeaseLost(ref wf) => {
                        warn!(workflow = %wf, "run abandoned after losing its lease")
                    }
                    RunOutcome::Failed(ref wf) | RunOutcome::Timeout(ref wf) => {
                        if tracker.record_panic(wf) {
                            warn!(workflow = %wf, "workflow flagged as poison pill after consecutive failures");
                        }
                    }
                    RunOutcome::Panicked(ref wf) => {
                        if tracker.record_panic(wf) {
                            error!(workflow = %wf, "workflow flagged as poison pill after consecutive panics");
                        }
                    }
                }
            }

            let run = self
                .engine
                .store()
                .pick_next_pending(Some(LeaseRequest {
                    worker_id: self.worker_id.clone(),
                    ttl: self.lease_ttl,
                }))
                .await;

            match run {
                Ok(Some(run)) => {
                    #[cfg(feature = "prometheus")]
                    counter!(WORKER_POLLS_TOTAL, "result" => "hit").increment(1);

                    // Poison pill check: skip workflows that keep failing
                    let is_blocked = {
                        let tracker = poison_tracker.lock().expect("poison tracker lock poisoned");
                        tracker.is_blocked(&run.workflow_name)
                    };
                    if is_blocked {
                        warn!(
                            workflow = %run.workflow_name,
                            run_id = %run.id,
                            "skipping run: workflow flagged as poison pill, marking as failed"
                        );
                        if let Err(e) = self
                            .engine
                            .store()
                            .update_run_status(run.id, RunStatus::Failed)
                            .await
                        {
                            error!(run_id = %run.id, error = %e, "failed to mark poisoned run as failed");
                        }
                        continue;
                    }

                    let permit = semaphore
                        .clone()
                        .acquire_owned()
                        .await
                        .map_err(|_| WorkerError::Internal("semaphore closed".to_string()))?;

                    idle_streak = 0;
                    let engine = self.engine.clone();
                    let run_id = run.id;
                    let workflow = run.workflow_name.clone();
                    let workflow_for_watcher = workflow.clone();
                    let run_timeout = self.run_timeout;

                    info!(run_id = %run_id, workflow = %workflow, "executing run");

                    #[cfg(feature = "prometheus")]
                    gauge!(WORKER_ACTIVE).increment(1.0);

                    // Keep the lease alive for as long as this run executes, and
                    // abandon the run as soon as the lease is lost.
                    let lease_token = CancellationToken::new();
                    let refresher = spawn(refresh_lease(
                        self.engine.store().clone(),
                        run_id,
                        LeaseRequest {
                            worker_id: self.worker_id.clone(),
                            ttl: self.lease_ttl,
                        },
                        self.lease_refresh_interval,
                        lease_token.clone(),
                    ));

                    let handle = spawn(async move {
                        let _permit = permit;
                        let result = tokio::select! {
                            biased;
                            _ = lease_token.cancelled() => {
                                refresher.abort();
                                // Dropping the execution future here cancels it;
                                // child processes die with it (kill_on_drop).
                                warn!(
                                    run_id = %run_id,
                                    workflow = %workflow,
                                    "abandoning run: worker lease lost"
                                );
                                #[cfg(feature = "prometheus")]
                                counter!(WORKER_LEASES_LOST_TOTAL).increment(1);
                                // No status write: the run belongs to someone else now.
                                return RunOutcome::LeaseLost(workflow);
                            }
                            result = timeout(run_timeout, engine.execute_handler_run(run_id)) => result,
                        };
                        refresher.abort();

                        match result {
                            Ok(Ok(_)) => {
                                info!(run_id = %run_id, workflow = %workflow, "run completed");
                                RunOutcome::Success(workflow)
                            }
                            Ok(Err(e)) => {
                                error!(run_id = %run_id, workflow = %workflow, error = %e, "run failed");
                                if let Err(store_err) = engine
                                    .store()
                                    .update_run(
                                        run_id,
                                        RunUpdate {
                                            status: Some(RunStatus::Failed),
                                            error: Some(e.to_string()),
                                            ..RunUpdate::default()
                                        },
                                    )
                                    .await
                                {
                                    error!(run_id = %run_id, error = %store_err, "failed to mark run as failed");
                                }
                                if let Err(cleanup_err) = engine
                                    .fail_orphaned_steps(run_id, "parent run failed")
                                    .await
                                {
                                    error!(run_id = %run_id, error = %cleanup_err, "failed to cleanup orphaned steps");
                                }
                                RunOutcome::Failed(workflow)
                            }
                            Err(_) => {
                                error!(
                                    run_id = %run_id,
                                    workflow = %workflow,
                                    timeout_secs = run_timeout.as_secs(),
                                    "run timed out"
                                );
                                let timeout_msg =
                                    format!("run timed out after {}s", run_timeout.as_secs());
                                if let Err(e) = engine
                                    .store()
                                    .update_run(
                                        run_id,
                                        RunUpdate {
                                            status: Some(RunStatus::Failed),
                                            error: Some(timeout_msg),
                                            ..RunUpdate::default()
                                        },
                                    )
                                    .await
                                {
                                    error!(run_id = %run_id, error = %e, "failed to mark timed-out run as failed");
                                }
                                if let Err(e) = engine
                                    .fail_orphaned_steps(run_id, "parent run timed out")
                                    .await
                                {
                                    error!(run_id = %run_id, error = %e, "failed to cleanup orphaned steps after timeout");
                                }
                                RunOutcome::Timeout(workflow)
                            }
                        }
                    });

                    // Spawn a watcher to catch panics and report outcomes
                    let watcher_engine = self.engine.clone();
                    let tx = outcome_tx.clone();
                    spawn(async move {
                        match handle.await {
                            Ok(outcome) => {
                                let _ = tx.send(outcome);
                            }
                            Err(e) => {
                                error!(run_id = %run_id, "spawned task panicked: {e}");
                                if let Err(store_err) = watcher_engine
                                    .store()
                                    .update_run_status(run_id, RunStatus::Failed)
                                    .await
                                {
                                    error!(run_id = %run_id, error = %store_err, "failed to mark panicked run as failed");
                                }
                                if let Err(cleanup_err) = watcher_engine
                                    .fail_orphaned_steps(run_id, "parent run panicked")
                                    .await
                                {
                                    error!(run_id = %run_id, error = %cleanup_err, "failed to cleanup orphaned steps after panic");
                                }
                                let _ = tx.send(RunOutcome::Panicked(workflow_for_watcher));
                            }
                        }
                        #[cfg(feature = "prometheus")]
                        gauge!(WORKER_ACTIVE).decrement(1.0);
                    });
                }
                Ok(None) => {
                    #[cfg(feature = "prometheus")]
                    counter!(WORKER_POLLS_TOTAL, "result" => "miss").increment(1);

                    idle_streak += 1;
                    let backoff = if idle_streak > 10 {
                        self.poll_interval * 3
                    } else if idle_streak > 5 {
                        self.poll_interval * 2
                    } else {
                        self.poll_interval
                    };
                    sleep(backoff).await;
                }
                Err(e) => {
                    warn!(error = %e, "poll error");
                    sleep(self.poll_interval).await;
                }
            }
        }

        // Graceful drain: wait for all in-flight tasks to release their permits
        info!(
            in_flight = self.concurrency - semaphore.available_permits(),
            "waiting for in-flight runs to complete..."
        );
        let _ = semaphore
            .acquire_many(self.concurrency as u32)
            .await
            .map_err(|_| WorkerError::Shutdown("semaphore closed during drain".to_string()))?;

        info!("all in-flight runs completed, worker shut down");
        Ok(())
    }
}

/// Outcome of a single run execution, used for poison pill tracking.
enum RunOutcome {
    /// Run completed successfully.
    Success(String),
    /// Run failed with an error (engine returned Err).
    Failed(String),
    /// Run exceeded its timeout.
    Timeout(String),
    /// Run panicked (task JoinError).
    Panicked(String),
    /// Run was abandoned because this worker lost its lease.
    LeaseLost(String),
}

/// Keep a run's lease alive until the run finishes or the lease is lost.
///
/// Cancels `lease_token` when the API hands the run to another worker, or when
/// refreshes keep failing for longer than the lease TTL — in both cases this
/// worker must stop executing the run rather than risk a double execution.
async fn refresh_lease(
    store: Arc<dyn Store>,
    run_id: uuid::Uuid,
    lease: LeaseRequest,
    refresh_interval: Duration,
    lease_token: CancellationToken,
) {
    let ttl = lease.ttl;
    let mut deadline = Instant::now() + ttl;

    loop {
        sleep(refresh_interval).await;

        match store.renew_lease(run_id, lease.clone()).await {
            Ok(_) => {
                deadline = Instant::now() + ttl;
            }
            Err(StoreError::LeaseLost { held_by, .. }) => {
                warn!(
                    run_id = %run_id,
                    held_by = held_by.as_deref().unwrap_or("unknown"),
                    "lease taken over by another worker"
                );
                lease_token.cancel();
                return;
            }
            Err(err) if Instant::now() >= deadline => {
                // The API has been unreachable longer than the lease lasts:
                // another worker may already have picked the run up.
                warn!(
                    run_id = %run_id,
                    error = %err,
                    ttl_secs = ttl.as_secs(),
                    "lease could not be refreshed before it expired"
                );
                lease_token.cancel();
                return;
            }
            Err(err) => {
                warn!(run_id = %run_id, error = %err, "lease refresh failed, retrying");
            }
        }
    }
}

/// Wait for SIGTERM or SIGINT (Ctrl+C).
async fn shutdown_signal() {
    use tokio::signal;

    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{SignalKind, signal};

        signal(SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = {
        use std::future::pending;
        pending::<()>()
    };

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironflow_core::providers::claude::ClaudeCodeProvider;

    #[test]
    fn builder_new_creates_default_config() {
        let builder = WorkerBuilder::new("http://localhost:3000", "my-token");
        assert_eq!(builder.api_url, "http://localhost:3000");
        assert_eq!(builder.worker_token, "my-token");
        assert_eq!(builder.concurrency, DEFAULT_CONCURRENCY);
        assert_eq!(builder.poll_interval, DEFAULT_POLL_INTERVAL);
        assert_eq!(builder.run_timeout, DEFAULT_RUN_TIMEOUT);
        assert_eq!(
            builder.max_consecutive_panics,
            DEFAULT_MAX_CONSECUTIVE_PANICS
        );
        assert_eq!(builder.panic_cooldown, DEFAULT_PANIC_COOLDOWN);
        assert!(builder.provider.is_none());
    }

    #[test]
    fn builder_with_trailing_slash_normalized() {
        let builder = WorkerBuilder::new("http://localhost:3000/", "token");
        assert_eq!(builder.api_url, "http://localhost:3000/");
    }

    #[test]
    fn builder_provider_sets_provider() {
        let provider = Arc::new(ClaudeCodeProvider::new());
        let builder =
            WorkerBuilder::new("http://localhost:3000", "token").provider(provider.clone());
        assert!(builder.provider.is_some());
    }

    #[test]
    fn builder_concurrency_sets_concurrency() {
        let builder = WorkerBuilder::new("http://localhost:3000", "token").concurrency(8);
        assert_eq!(builder.concurrency, 8);
    }

    #[test]
    fn builder_concurrency_zero_accepted() {
        let provider = Arc::new(ClaudeCodeProvider::new());
        let builder = WorkerBuilder::new("http://localhost:3000", "token")
            .provider(provider)
            .concurrency(0);
        assert_eq!(builder.concurrency, 0);
    }

    #[test]
    fn builder_poll_interval_sets_interval() {
        let interval = Duration::from_secs(5);
        let builder = WorkerBuilder::new("http://localhost:3000", "token").poll_interval(interval);
        assert_eq!(builder.poll_interval, interval);
    }

    #[test]
    fn builder_run_timeout_sets_timeout() {
        let dur = Duration::from_secs(120);
        let builder = WorkerBuilder::new("http://localhost:3000", "token").run_timeout(dur);
        assert_eq!(builder.run_timeout, dur);
    }

    #[test]
    fn builder_max_consecutive_panics_sets_value() {
        let builder =
            WorkerBuilder::new("http://localhost:3000", "token").max_consecutive_panics(10);
        assert_eq!(builder.max_consecutive_panics, 10);
    }

    #[test]
    fn builder_panic_cooldown_sets_value() {
        let dur = Duration::from_secs(600);
        let builder = WorkerBuilder::new("http://localhost:3000", "token").panic_cooldown(dur);
        assert_eq!(builder.panic_cooldown, dur);
    }

    #[test]
    fn builder_defaults_lease_settings() {
        let builder = WorkerBuilder::new("http://localhost:3000", "token");
        assert_eq!(builder.lease_ttl, DEFAULT_LEASE_TTL);
        assert_eq!(
            builder.lease_refresh_interval,
            DEFAULT_LEASE_REFRESH_INTERVAL
        );
        assert!(builder.worker_id.starts_with("worker-"));
    }

    #[test]
    fn builder_generates_a_distinct_worker_id_per_instance() {
        let a = WorkerBuilder::new("http://localhost:3000", "token");
        let b = WorkerBuilder::new("http://localhost:3000", "token");
        assert_ne!(a.worker_id, b.worker_id);
    }

    #[test]
    fn builder_worker_id_overrides_default() {
        let builder = WorkerBuilder::new("http://localhost:3000", "token").worker_id("worker-eu-1");
        assert_eq!(builder.worker_id, "worker-eu-1");
    }

    #[test]
    fn builder_lease_ttl_sets_value() {
        let dur = Duration::from_secs(120);
        let builder = WorkerBuilder::new("http://localhost:3000", "token").lease_ttl(dur);
        assert_eq!(builder.lease_ttl, dur);
    }

    #[test]
    fn builder_lease_refresh_interval_sets_value() {
        let dur = Duration::from_secs(5);
        let builder =
            WorkerBuilder::new("http://localhost:3000", "token").lease_refresh_interval(dur);
        assert_eq!(builder.lease_refresh_interval, dur);
    }

    #[test]
    fn builder_build_without_provider_fails() {
        let builder = WorkerBuilder::new("http://localhost:3000", "token");
        let result = builder.build();
        assert!(result.is_err());
        match result {
            Err(WorkerError::Internal(msg)) => {
                assert!(msg.contains("provider is required"));
            }
            _ => panic!("expected Internal error about missing provider"),
        }
    }

    #[test]
    fn builder_build_with_provider_succeeds() {
        let provider = Arc::new(ClaudeCodeProvider::new());
        let builder = WorkerBuilder::new("http://localhost:3000", "token").provider(provider);
        let result = builder.build();
        assert!(result.is_ok());
    }

    #[test]
    fn builder_build_creates_worker_with_correct_concurrency() {
        let provider = Arc::new(ClaudeCodeProvider::new());
        let builder = WorkerBuilder::new("http://localhost:3000", "token")
            .provider(provider)
            .concurrency(16);
        let worker = builder.build().unwrap();
        assert_eq!(worker.concurrency, 16);
    }

    #[test]
    fn builder_build_creates_worker_with_correct_interval() {
        let provider = Arc::new(ClaudeCodeProvider::new());
        let interval = Duration::from_secs(10);
        let builder = WorkerBuilder::new("http://localhost:3000", "token")
            .provider(provider)
            .poll_interval(interval);
        let worker = builder.build().unwrap();
        assert_eq!(worker.poll_interval, interval);
    }

    #[test]
    fn builder_build_preserves_timeout() {
        let provider = Arc::new(ClaudeCodeProvider::new());
        let dur = Duration::from_secs(300);
        let worker = WorkerBuilder::new("http://localhost:3000", "token")
            .provider(provider)
            .run_timeout(dur)
            .build()
            .unwrap();
        assert_eq!(worker.run_timeout, dur);
    }

    #[test]
    fn builder_build_preserves_poison_pill_config() {
        let provider = Arc::new(ClaudeCodeProvider::new());
        let cooldown = Duration::from_secs(120);
        let worker = WorkerBuilder::new("http://localhost:3000", "token")
            .provider(provider)
            .max_consecutive_panics(7)
            .panic_cooldown(cooldown)
            .build()
            .unwrap();
        assert_eq!(worker.max_consecutive_panics, 7);
        assert_eq!(worker.panic_cooldown, cooldown);
    }

    #[test]
    fn builder_chaining_works() {
        let provider = Arc::new(ClaudeCodeProvider::new());
        let result = WorkerBuilder::new("http://localhost:3000", "token")
            .provider(provider)
            .concurrency(4)
            .poll_interval(Duration::from_secs(3))
            .run_timeout(Duration::from_secs(600))
            .max_consecutive_panics(5)
            .panic_cooldown(Duration::from_secs(120))
            .build();
        assert!(result.is_ok());
        let worker = result.unwrap();
        assert_eq!(worker.concurrency, 4);
        assert_eq!(worker.poll_interval, Duration::from_secs(3));
        assert_eq!(worker.run_timeout, Duration::from_secs(600));
        assert_eq!(worker.max_consecutive_panics, 5);
        assert_eq!(worker.panic_cooldown, Duration::from_secs(120));
    }

    #[test]
    fn builder_empty_api_url_accepted() {
        let provider = Arc::new(ClaudeCodeProvider::new());
        let builder = WorkerBuilder::new("", "token").provider(provider);
        let result = builder.build();
        assert!(result.is_ok());
    }

    #[test]
    fn builder_empty_token_accepted() {
        let provider = Arc::new(ClaudeCodeProvider::new());
        let builder = WorkerBuilder::new("http://localhost:3000", "").provider(provider);
        let result = builder.build();
        assert!(result.is_ok());
    }

    #[cfg(feature = "heartbeat")]
    #[test]
    fn builder_heartbeat_defaults() {
        let builder = WorkerBuilder::new("http://localhost:3000", "token");
        assert!(builder.heartbeat_url.is_none());
        assert_eq!(builder.heartbeat_interval, DEFAULT_HEARTBEAT_INTERVAL);
    }

    #[cfg(feature = "heartbeat")]
    #[test]
    fn builder_heartbeat_url_sets_url() {
        let builder = WorkerBuilder::new("http://localhost:3000", "token")
            .heartbeat_url("https://uptime.betterstack.com/api/v1/heartbeat/abc");
        assert_eq!(
            builder.heartbeat_url.as_deref(),
            Some("https://uptime.betterstack.com/api/v1/heartbeat/abc")
        );
    }

    #[cfg(feature = "heartbeat")]
    #[test]
    fn builder_heartbeat_custom_interval() {
        let interval = Duration::from_secs(10);
        let builder =
            WorkerBuilder::new("http://localhost:3000", "token").heartbeat_interval(interval);
        assert_eq!(builder.heartbeat_interval, interval);
    }

    #[cfg(feature = "heartbeat")]
    #[test]
    fn builder_build_preserves_heartbeat_config() {
        let provider = Arc::new(ClaudeCodeProvider::new());
        let interval = Duration::from_secs(15);
        let worker = WorkerBuilder::new("http://localhost:3000", "token")
            .provider(provider)
            .heartbeat_url("https://example.com/heartbeat")
            .heartbeat_interval(interval)
            .build()
            .unwrap();
        assert_eq!(
            worker.heartbeat_url.as_deref(),
            Some("https://example.com/heartbeat")
        );
        assert_eq!(worker.heartbeat_interval, interval);
    }

    #[cfg(feature = "heartbeat")]
    #[test]
    fn builder_build_without_heartbeat_url_has_none() {
        let provider = Arc::new(ClaudeCodeProvider::new());
        let worker = WorkerBuilder::new("http://localhost:3000", "token")
            .provider(provider)
            .build()
            .unwrap();
        assert!(worker.heartbeat_url.is_none());
    }

    // --- PoisonPillTracker tests ---

    #[test]
    fn poison_tracker_not_blocked_initially() {
        let tracker = PoisonPillTracker::new(3, Duration::from_secs(300));
        assert!(!tracker.is_blocked("my-workflow"));
    }

    #[test]
    fn poison_tracker_blocked_after_max_panics() {
        let mut tracker = PoisonPillTracker::new(3, Duration::from_secs(300));
        assert!(!tracker.record_panic("wf"));
        assert!(!tracker.record_panic("wf"));
        assert!(tracker.record_panic("wf"));
        assert!(tracker.is_blocked("wf"));
    }

    #[test]
    fn poison_tracker_success_resets_count() {
        let mut tracker = PoisonPillTracker::new(3, Duration::from_secs(300));
        tracker.record_panic("wf");
        tracker.record_panic("wf");
        tracker.record_success("wf");
        assert!(!tracker.is_blocked("wf"));
        // After reset, need 3 more panics to block
        assert!(!tracker.record_panic("wf"));
    }

    #[test]
    fn poison_tracker_independent_per_workflow() {
        let mut tracker = PoisonPillTracker::new(2, Duration::from_secs(300));
        tracker.record_panic("wf-a");
        tracker.record_panic("wf-a");
        assert!(tracker.is_blocked("wf-a"));
        assert!(!tracker.is_blocked("wf-b"));
    }

    #[test]
    fn poison_tracker_unblocks_after_cooldown() {
        let mut tracker = PoisonPillTracker::new(2, Duration::from_millis(0));
        tracker.record_panic("wf");
        tracker.record_panic("wf");
        // Cooldown is 0ms, should immediately unblock
        assert!(!tracker.is_blocked("wf"));
    }
}
