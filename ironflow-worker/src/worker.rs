//! Worker — polls the API for pending runs and executes them.

use std::sync::Arc;
use std::time::Duration;

use tokio::spawn;
use tokio::sync::Semaphore;
use tokio::time::sleep;
use tracing::{error, info, warn};

use ironflow_core::provider::AgentProvider;
#[cfg(feature = "prometheus")]
use ironflow_core::metric_names::{WORKER_ACTIVE, WORKER_POLLS_TOTAL};
#[cfg(feature = "prometheus")]
use metrics::{counter, gauge};
use ironflow_engine::engine::Engine;
use ironflow_engine::handler::WorkflowHandler;
use ironflow_store::store::RunStore;

use crate::api_store::ApiRunStore;
use crate::error::WorkerError;

const DEFAULT_CONCURRENCY: usize = 2;
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(2);

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

        Ok(Worker {
            engine: Arc::new(engine),
            concurrency: self.concurrency,
            poll_interval: self.poll_interval,
        })
    }
}

/// Background worker that polls the API and executes workflows.
pub struct Worker {
    engine: Arc<Engine>,
    concurrency: usize,
    poll_interval: Duration,
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
}
