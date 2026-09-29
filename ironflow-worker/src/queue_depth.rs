//! `ironflow_worker_queue_depth`: the number of `Pending` runs, published by
//! the worker.
//!
//! The worker's store is the API, which answers no stats query: the count
//! comes from the internal `GET /runs/pending-count` route.

use std::time::{Duration, Instant};

use metrics::gauge;
use reqwest::Client;
use serde::Deserialize;
use tracing::debug;

use ironflow_core::metric_names::WORKER_QUEUE_DEPTH;

use crate::error::WorkerError;

/// How often the gauge is refreshed.
const REFRESH_INTERVAL: Duration = Duration::from_secs(5);

/// Refreshes the queue depth gauge from the API server.
pub(crate) struct QueueDepthGauge {
    client: Client,
    url: String,
    token: String,
    last_refresh: Option<Instant>,
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
        }
    }

    /// Refresh the gauge on the first call, then at most once per
    /// [`REFRESH_INTERVAL`].
    ///
    /// A failure leaves the gauge as it was and is logged at `debug`: the
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

        match self.pending_runs().await {
            Ok(count) => gauge!(WORKER_QUEUE_DEPTH).set(count as f64),
            Err(e) => debug!(error = %e, "queue depth refresh failed"),
        }
    }

    /// Number of `Pending` runs, as counted by the API server.
    ///
    /// # Errors
    ///
    /// Returns [`WorkerError::Internal`] on a transport error, a non-2xx
    /// answer or a body that is not the expected envelope.
    async fn pending_runs(&self) -> Result<u64, WorkerError> {
        #[derive(Deserialize)]
        struct Envelope {
            data: PendingCount,
        }

        #[derive(Deserialize)]
        struct PendingCount {
            pending_runs: u64,
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
        Ok(envelope.data.pending_runs)
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
    use ironflow_store::models::{NewRun, TriggerKind};
    use ironflow_store::store::RunStore;
    use serde_json::json;
    use tokio::net::TcpListener;
    use tokio::spawn;
    use tokio::sync::broadcast;

    use super::*;

    /// Serve the real API router over TCP with `pending` pending runs.
    async fn spawn_api(pending: usize) -> String {
        let store = Arc::new(InMemoryStore::new());
        for _ in 0..pending {
            store
                .create_run(NewRun {
                    created_by: None,
                    workflow_name: "queued".to_string(),
                    trigger: TriggerKind::Manual,
                    payload: json!({}),
                    max_retries: 0,
                    handler_version: None,
                    labels: HashMap::new(),
                    scheduled_at: None,
                    idempotency_key: None,
                    max_cost_usd: None,
                })
                .await
                .unwrap();
        }
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

        assert_eq!(gauge.pending_runs().await.unwrap(), 2);
    }

    #[tokio::test]
    async fn pending_runs_fails_on_a_rejected_token() {
        let api_url = spawn_api(2).await;
        let gauge = QueueDepthGauge::new(&api_url, "wrong-token");

        let err = gauge.pending_runs().await.unwrap_err().to_string();
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

        let err = gauge.pending_runs().await.unwrap_err().to_string();
        assert!(err.contains("queue depth request failed"), "{err}");
    }
}
