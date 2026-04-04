//! Worker — polls the API for pending runs and executes them.

use std::sync::Arc;
use std::time::Duration;

use tokio::spawn;
use tokio::sync::Semaphore;
use tokio::time::sleep;
use tracing::{error, info, warn};

#[cfg(feature = "prometheus")]
use ironflow_core::metric_names::{WORKER_ACTIVE, WORKER_POLLS_TOTAL};
use ironflow_core::provider::AgentProvider;
use ironflow_engine::engine::Engine;
use ironflow_engine::handler::WorkflowHandler;
use ironflow_store::store::RunStore;
#[cfg(feature = "prometheus")]
use metrics::{counter, gauge};
#[cfg(feature = "heartbeat")]
use reqwest::Client;

use crate::api_store::ApiRunStore;
use crate::error::WorkerError;

const DEFAULT_CONCURRENCY: usize = 2;
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(2);
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
///     .build()?;
///
/// worker.run().await?;
/// # Ok(())
/// # }
/// ```
pub struct WorkerBuilder {
    api_url: String,
    worker_token: String,
    provider: Option<Arc<dyn AgentProvider>>,
    handlers: Vec<Box<dyn WorkflowHandler>>,
    concurrency: usize,
    poll_interval: Duration,
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
            provider: None,
            handlers: Vec::new(),
            concurrency: DEFAULT_CONCURRENCY,
            poll_interval: DEFAULT_POLL_INTERVAL,
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

        let store: Arc<dyn RunStore> =
            Arc::new(ApiRunStore::new(&self.api_url, &self.worker_token));

        let mut engine = Engine::new(store, provider);
        for handler in self.handlers {
            engine
                .register_boxed(handler)
                .map_err(WorkerError::Engine)?;
        }

        #[cfg(feature = "heartbeat")]
        let heartbeat_client = Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("failed to build heartbeat HTTP client");

        Ok(Worker {
            engine: Arc::new(engine),
            concurrency: self.concurrency,
            poll_interval: self.poll_interval,
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
    concurrency: usize,
    poll_interval: Duration,
    #[cfg(feature = "heartbeat")]
    heartbeat_url: Option<String>,
    #[cfg(feature = "heartbeat")]
    heartbeat_interval: Duration,
    #[cfg(feature = "heartbeat")]
    heartbeat_client: Client,
}

impl Worker {
    /// Run the worker loop. Blocks until an error occurs or the process exits.
    ///
    /// # Errors
    ///
    /// Returns [`WorkerError`] if the polling loop encounters an unrecoverable error.
    pub async fn run(&self) -> Result<(), WorkerError> {
        let semaphore = Arc::new(Semaphore::new(self.concurrency));
        let mut idle_streak = 0u32;

        info!(
            concurrency = self.concurrency,
            poll_interval_ms = self.poll_interval.as_millis() as u64,
            "worker started"
        );

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

        loop {
            let run = self.engine.store().pick_next_pending().await;

            match run {
                Ok(Some(run)) => {
                    #[cfg(feature = "prometheus")]
                    counter!(WORKER_POLLS_TOTAL, "result" => "hit").increment(1);

                    let permit = semaphore
                        .clone()
                        .acquire_owned()
                        .await
                        .map_err(|_| WorkerError::Internal("semaphore closed".to_string()))?;

                    idle_streak = 0;
                    let engine = self.engine.clone();
                    let run_id = run.id;
                    let workflow = run.workflow_name.clone();

                    info!(run_id = %run_id, workflow = %workflow, "executing run");

                    #[cfg(feature = "prometheus")]
                    gauge!(WORKER_ACTIVE).increment(1.0);

                    let handle = spawn(async move {
                        let _permit = permit;
                        match engine.execute_handler_run(run_id).await {
                            Ok(_) => {
                                info!(run_id = %run_id, workflow = %workflow, "run completed");
                            }
                            Err(e) => {
                                error!(run_id = %run_id, workflow = %workflow, error = %e, "run failed");
                            }
                        }
                        #[cfg(feature = "prometheus")]
                        gauge!(WORKER_ACTIVE).decrement(1.0);
                    });

                    // Spawn a watcher to catch panics and mark the run as failed
                    let store = self.engine.store().clone();
                    spawn(async move {
                        if let Err(e) = handle.await {
                            error!(run_id = %run_id, "spawned task panicked: {e}");
                            if let Err(store_err) = store
                                .update_run_status(
                                    run_id,
                                    ironflow_store::entities::RunStatus::Failed,
                                )
                                .await
                            {
                                error!(run_id = %run_id, error = %store_err, "failed to mark panicked run as failed");
                            }
                        }
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
    fn builder_chaining_works() {
        let provider = Arc::new(ClaudeCodeProvider::new());
        let result = WorkerBuilder::new("http://localhost:3000", "token")
            .provider(provider)
            .concurrency(4)
            .poll_interval(Duration::from_secs(3))
            .build();
        assert!(result.is_ok());
        let worker = result.unwrap();
        assert_eq!(worker.concurrency, 4);
        assert_eq!(worker.poll_interval, Duration::from_secs(3));
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
}
