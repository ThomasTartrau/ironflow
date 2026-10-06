//! `ironflow_worker_queue_depth`: the number of `Pending` runs, and
//! `ironflow_worker_queue_blocked_runs`: the due runs held back by each
//! saturated concurrency group, published by the worker.
//!
//! The worker's store is the API, which answers no stats query: the
//! counts come from the internal `GET /runs/pending-count` route.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use metrics::gauge;
use reqwest::Client;
use serde::Deserialize;
use tracing::debug;

use ironflow_core::metric_names::{WORKER_QUEUE_BLOCKED_RUNS, WORKER_QUEUE_DEPTH};

use crate::error::WorkerError;

/// How often the gauge is refreshed.
const REFRESH_INTERVAL: Duration = Duration::from_secs(5);

/// Queue counts read from the API server.
#[derive(Debug, Deserialize)]
struct QueueSnapshot {
    pending_runs: u64,
    /// Absent when the API predates concurrency groups.
    blocked_by_group: Option<Vec<GroupBacklog>>,
}

/// Due runs held back by one saturated concurrency group.
#[derive(Debug, Deserialize, PartialEq)]
struct GroupBacklog {
    group: String,
    blocked_runs: u64,
}

/// Refreshes the queue gauges from the API server.
pub(crate) struct QueueDepthGauge {
    client: Client,
    url: String,
    token: String,
    last_refresh: Option<Instant>,
    /// Groups given a non-zero value at the last refresh, so a group that
    /// is no longer saturated drops back to zero instead of keeping a stale
    /// value.
    blocked_groups: HashSet<String>,
}

impl QueueDepthGauge {
    /// Build a gauge that reads the count from the API at `api_url`.
    ///
    /// # Panics
    ///
    /// Panics if the HTTP client cannot be built (TLS backend unavailable).
    pub(crate) fn new(api_url: &str, token: &str) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("failed to build queue depth HTTP client");

        Self {
            client,
            url: format!(
                "{}/api/v1/internal/runs/pending-count",
                api_url.trim_end_matches('/')
            ),
            token: token.to_string(),
            last_refresh: None,
            blocked_groups: HashSet::new(),
        }
    }

    /// Refresh the gauges on the first call, then at most once per
    /// [`REFRESH_INTERVAL`].
    ///
    /// A failure leaves the gauges as they were and is logged at `debug`: the
    /// poll loop calls this every few seconds, and an API that predates the
    /// route would otherwise flood the logs.
    pub(crate) async fn refresh_if_due(&mut self) {
        if self
            .last_refresh
            .is_some_and(|at| at.elapsed() < REFRESH_INTERVAL)
        {
            return;
        }
        self.last_refresh = Some(Instant::now());

        match self.snapshot().await {
            Ok(snapshot) => {
                gauge!(WORKER_QUEUE_DEPTH).set(snapshot.pending_runs as f64);
                if let Some(blocked) = snapshot.blocked_by_group {
                    self.publish_blocked(blocked);
                }
            }
            Err(e) => debug!(error = %e, "queue depth refresh failed"),
        }
    }

    /// Set the blocked-runs gauge of every saturated group, and zero the
    /// groups that were saturated at the previous refresh but no longer are.
    fn publish_blocked(&mut self, blocked: Vec<GroupBacklog>) {
        let current: HashSet<String> = blocked.iter().map(|b| b.group.clone()).collect();
        for group in self.blocked_groups.difference(&current) {
            gauge!(WORKER_QUEUE_BLOCKED_RUNS, "group" => group.clone()).set(0.0);
        }
        for backlog in blocked {
            gauge!(WORKER_QUEUE_BLOCKED_RUNS, "group" => backlog.group)
                .set(backlog.blocked_runs as f64);
        }
        self.blocked_groups = current;
    }

    /// Number of `Pending` runs and of runs blocked per group, as counted by
    /// the API server.
    ///
    /// # Errors
    ///
    /// Returns [`WorkerError::Internal`] on a transport error, a non-2xx
    /// answer or a body that is not the expected envelope.
    async fn snapshot(&self) -> Result<QueueSnapshot, WorkerError> {
        #[derive(Deserialize)]
        struct Envelope {
            data: QueueSnapshot,
        }

        let resp = self
            .client
            .get(&self.url)
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|e| WorkerError::Internal(format!("queue depth request failed: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.map_err(|e| {
                WorkerError::Internal(format!("queue depth API answered {status}: {e}"))
            })?;
            return Err(WorkerError::Internal(format!(
                "queue depth API answered {status}: {body}"
            )));
        }

        let envelope: Envelope = resp
            .json()
            .await
            .map_err(|e| WorkerError::Internal(format!("queue depth response is invalid: {e}")))?;
        Ok(envelope.data)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use axum::serve;
    use ironflow_api::routes::{RouterConfig, create_router};
    use ironflow_api::state::AppState;
    use ironflow_auth::jwt::JwtConfig;
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::notify::Event;
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::models::{ConcurrencyLimit, NewRun, RunStatus, TriggerKind};
    use ironflow_store::store::RunStore;
    use serde_json::{from_value, json};
    use tokio::net::TcpListener;
    use tokio::spawn;
    use tokio::sync::broadcast;

    use super::*;

    fn new_run(concurrency_limits: Vec<ConcurrencyLimit>) -> NewRun {
        NewRun {
            created_by: None,
            workflow_name: "queued".to_string(),
            trigger: TriggerKind::Manual,
            payload: json!({}),
            max_retries: 0,
            handler_version: None,
            labels: HashMap::new(),
            scheduled_at: None,
            idempotency_key: None,
            concurrency_key: None,
            priority: 0,
            concurrency_limits,
            max_cost_usd: None,
        }
    }

    /// Serve the real API router over TCP with `pending` pending runs.
    async fn spawn_api(pending: usize) -> String {
        let store = Arc::new(InMemoryStore::new());
        for _ in 0..pending {
            store.create_run(new_run(Vec::new())).await.unwrap();
        }
        serve_store(store).await
    }

    /// Serve the real API router over TCP on top of `store`.
    async fn serve_store(store: Arc<InMemoryStore>) -> String {
        let engine = Engine::new(store.clone(), Arc::new(ClaudeCodeProvider::new()));
        let jwt_config = Arc::new(JwtConfig {
            secret: "test-secret".to_string(),
            access_token_ttl_secs: 900,
            refresh_token_ttl_secs: 604800,
            cookie_domain: None,
            cookie_secure: false,
        });
        let (event_sender, _) = broadcast::channel::<Event>(1);
        let state = AppState::new(
            store,
            Arc::new(engine),
            jwt_config,
            "test-worker-token".to_string(),
            event_sender,
        );
        let config = RouterConfig {
            rate_limit_auth: None,
            rate_limit_general: None,
            ..RouterConfig::default()
        };
        let router = create_router(state, config);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        spawn(async move {
            serve(listener, router).await.unwrap();
        });
        format!("http://{addr}/")
    }

    #[tokio::test]
    async fn pending_runs_reads_the_api_count() {
        let api_url = spawn_api(2).await;
        let gauge = QueueDepthGauge::new(&api_url, "test-worker-token");

        let snapshot = gauge.snapshot().await.unwrap();
        assert_eq!(snapshot.pending_runs, 2);
        assert_eq!(snapshot.blocked_by_group, Some(Vec::new()));
    }

    #[tokio::test]
    async fn snapshot_reads_runs_blocked_by_group() {
        let store = Arc::new(InMemoryStore::new());
        let limits = || vec![ConcurrencyLimit::new("repo:acme", 1)];
        let holder = store
            .create_run(new_run(limits()))
            .await
            .unwrap()
            .into_run();
        store
            .update_run_status(holder.id, RunStatus::Running)
            .await
            .unwrap();
        store.create_run(new_run(limits())).await.unwrap();
        let api_url = serve_store(store).await;
        let gauge = QueueDepthGauge::new(&api_url, "test-worker-token");

        let snapshot = gauge.snapshot().await.unwrap();
        assert_eq!(snapshot.pending_runs, 1);
        assert_eq!(
            snapshot.blocked_by_group,
            Some(vec![GroupBacklog {
                group: "repo:acme".to_string(),
                blocked_runs: 1,
            }])
        );
    }

    #[test]
    fn snapshot_accepts_an_api_without_concurrency_groups() {
        let snapshot: QueueSnapshot = from_value(json!({ "pending_runs": 4 })).unwrap();
        assert_eq!(snapshot.pending_runs, 4);
        assert_eq!(snapshot.blocked_by_group, None);
    }

    #[tokio::test]
    async fn pending_runs_fails_on_a_rejected_token() {
        let api_url = spawn_api(2).await;
        let gauge = QueueDepthGauge::new(&api_url, "wrong-token");

        let err = gauge.snapshot().await.unwrap_err().to_string();
        assert!(err.contains("401"), "{err}");
        assert!(err.contains("INVALID_WORKER_TOKEN"), "{err}");
    }

    #[tokio::test]
    async fn pending_runs_fails_when_the_api_is_unreachable() {
        // Bind then drop: nothing listens on this port any more.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let gauge = QueueDepthGauge::new(&format!("http://{addr}"), "test-worker-token");

        let err = gauge.snapshot().await.unwrap_err().to_string();
        assert!(err.contains("queue depth request failed"), "{err}");
    }
}
