//! Retention-based purging of old runs and their artifacts.
//!
//! The [`RunPurger`] periodically removes terminal runs that exceed the
//! configured retention policy (by age or by per-workflow count) and deletes
//! their artifact blobs from the blob store.
//!
//! This is distinct from the [`Reaper`](crate::reaper::Reaper), which recovers
//! runs whose worker lease expired. The reaper keeps runs alive; the purger
//! removes the ones that are done and old.

use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use ironflow_artifacts::blob_store::BlobStore;
use ironflow_store::entities::{PurgePolicy, PurgeReason};
use ironflow_store::store::Store;
use tokio::time::interval;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

#[cfg(feature = "prometheus")]
use ironflow_core::metric_names::RUNS_PURGED_TOTAL;
#[cfg(feature = "prometheus")]
use metrics::counter;

/// How often the purger runs by default (once per day).
pub const DEFAULT_PURGE_INTERVAL: Duration = Duration::from_secs(86400);

/// How many runs a single tick processes.
pub const DEFAULT_PURGE_BATCH_SIZE: u32 = 100;

/// Periodic task that purges terminal runs exceeding the retention policy.
///
/// # Examples
///
/// ```no_run
/// use std::sync::Arc;
/// use std::time::Duration;
/// use ironflow_api::purger::RunPurger;
/// use ironflow_store::entities::PurgePolicy;
/// use ironflow_store::memory::InMemoryStore;
/// use ironflow_store::store::Store;
/// use tokio_util::sync::CancellationToken;
///
/// # async fn example() {
/// let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
/// let policy = PurgePolicy {
///     max_age_days: 90,
///     max_runs_per_workflow: 1000,
///     dry_run: false,
/// };
///
/// let purger = RunPurger::new(store, policy)
///     .interval(Duration::from_secs(3600));
/// tokio::spawn(purger.run(CancellationToken::new()));
/// # }
/// ```
pub struct RunPurger {
    store: Arc<dyn Store>,
    blob_store: Option<Arc<dyn BlobStore>>,
    policy: PurgePolicy,
    interval: Duration,
    batch_size: u32,
}

impl RunPurger {
    /// Create a purger with the default interval and batch size.
    pub fn new(store: Arc<dyn Store>, policy: PurgePolicy) -> Self {
        Self {
            store,
            blob_store: None,
            policy,
            interval: DEFAULT_PURGE_INTERVAL,
            batch_size: DEFAULT_PURGE_BATCH_SIZE,
        }
    }

    /// Set the blob store for artifact deletion.
    ///
    /// When `None`, artifact metadata is still removed but no blobs are deleted
    /// (they either do not exist or become orphans).
    pub fn with_blob_store(mut self, blob_store: Option<Arc<dyn BlobStore>>) -> Self {
        self.blob_store = blob_store;
        self
    }

    /// Set how often the purger runs.
    pub fn interval(mut self, interval: Duration) -> Self {
        self.interval = interval;
        self
    }

    /// Set how many runs a single tick processes.
    pub fn batch_size(mut self, batch_size: u32) -> Self {
        self.batch_size = batch_size;
        self
    }

    /// Run the purge loop until `shutdown` is cancelled.
    pub async fn run(self, shutdown: CancellationToken) {
        let mut ticker = interval(self.interval);
        ticker.tick().await;

        info!(
            interval_secs = self.interval.as_secs(),
            batch_size = self.batch_size,
            max_age_days = self.policy.max_age_days,
            max_runs_per_workflow = self.policy.max_runs_per_workflow,
            dry_run = self.policy.dry_run,
            "purger started"
        );

        loop {
            tokio::select! {
                _ = shutdown.cancelled() => {
                    info!("purger stopped");
                    return;
                }
                _ = ticker.tick() => {
                    self.tick().await;
                }
            }
        }
    }

    /// Purge one batch of eligible runs.
    ///
    /// Exposed for tests and for callers that drive the schedule themselves.
    pub async fn tick(&self) {
        let purgeable = match self
            .store
            .list_purgeable_runs(&self.policy, self.batch_size)
            .await
        {
            Ok(p) => p,
            Err(err) => {
                error!(error = %err, "failed to list purgeable runs");
                return;
            }
        };

        if purgeable.is_empty() {
            return;
        }

        if self.policy.dry_run {
            for entry in &purgeable {
                info!(
                    run_id = %entry.run_id,
                    workflow = %entry.workflow_name,
                    reason = %entry.reason,
                    "[dry-run] would purge run"
                );

                #[cfg(feature = "prometheus")]
                counter!(
                    RUNS_PURGED_TOTAL,
                    "workflow" => entry.workflow_name.clone(),
                    "reason" => entry.reason.to_string(),
                    "dry_run" => "true"
                )
                .increment(1);
            }

            return;
        }

        warn!(
            count = purgeable.len(),
            batch_size = self.batch_size,
            "purging old runs"
        );

        for entry in &purgeable {
            self.purge_run(entry.run_id, &entry.workflow_name, &entry.reason)
                .await;
        }
    }

    async fn purge_run(&self, run_id: Uuid, workflow_name: &str, reason: &PurgeReason) {
        match self.store.delete_run(run_id).await {
            Ok(storage_keys) => {
                if let Some(ref blob_store) = self.blob_store {
                    for key in &storage_keys {
                        if let Err(err) = blob_store.delete(key).await {
                            error!(
                                run_id = %run_id,
                                storage_key = %key,
                                error = %err,
                                "failed to delete artifact blob"
                            );
                        }
                    }
                }

                info!(
                    run_id = %run_id,
                    workflow = %workflow_name,
                    reason = %reason,
                    "purged run"
                );

                #[cfg(feature = "prometheus")]
                counter!(
                    RUNS_PURGED_TOTAL,
                    "workflow" => workflow_name.to_string(),
                    "reason" => reason.to_string(),
                    "dry_run" => "false"
                )
                .increment(1);
            }
            Err(err) => {
                error!(
                    run_id = %run_id,
                    workflow = %workflow_name,
                    error = %err,
                    "failed to delete run during purge"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::time::Duration;

    use chrono::{TimeDelta, Utc};
    use ironflow_store::entities::{NewRun, PurgePolicy, RunStatus, TriggerKind};
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::store::RunStore;
    use serde_json::json;
    use uuid::Uuid;

    use super::*;

    fn new_run(name: &str) -> NewRun {
        NewRun {
            workflow_name: name.to_string(),
            trigger: TriggerKind::Manual,
            payload: json!({}),
            max_retries: 0,
            handler_version: None,
            labels: HashMap::new(),
            scheduled_at: None,
            created_by: None,
            idempotency_key: None,
            max_cost_usd: None,
        }
    }

    async fn create_terminal_run(store: &InMemoryStore, name: &str, status: RunStatus) -> Uuid {
        let run = store.create_run(new_run(name)).await.unwrap().into_run();
        store
            .update_run_status(run.id, RunStatus::Running)
            .await
            .unwrap();
        store.update_run_status(run.id, status).await.unwrap();
        run.id
    }

    async fn backdate_run(store: &InMemoryStore, run_id: Uuid, days: i64) {
        store
            .set_run_created_at(run_id, Utc::now() - TimeDelta::days(days))
            .await;
    }

    fn build(store: Arc<InMemoryStore>, policy: PurgePolicy) -> RunPurger {
        let store_dyn: Arc<dyn Store> = store;
        RunPurger::new(store_dyn, policy)
    }

    #[tokio::test]
    async fn tick_purges_runs_older_than_max_age() {
        let store = Arc::new(InMemoryStore::new());
        let old_id = create_terminal_run(&store, "deploy", RunStatus::Completed).await;
        backdate_run(&store, old_id, 100).await;
        let recent_id = create_terminal_run(&store, "deploy", RunStatus::Completed).await;

        let policy = PurgePolicy {
            max_age_days: 90,
            max_runs_per_workflow: 10000,
            dry_run: false,
        };
        let purger = build(store.clone(), policy);
        purger.tick().await;

        assert!(store.get_run(old_id).await.unwrap().is_none());
        assert!(store.get_run(recent_id).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn tick_in_dry_run_does_not_delete() {
        let store = Arc::new(InMemoryStore::new());
        let old_id = create_terminal_run(&store, "deploy", RunStatus::Completed).await;
        backdate_run(&store, old_id, 100).await;

        let policy = PurgePolicy {
            max_age_days: 90,
            max_runs_per_workflow: 10000,
            dry_run: true,
        };
        let purger = build(store.clone(), policy);
        purger.tick().await;

        assert!(store.get_run(old_id).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn tick_does_not_purge_non_terminal_runs() {
        let store = Arc::new(InMemoryStore::new());
        let pending = store
            .create_run(new_run("deploy"))
            .await
            .unwrap()
            .into_run();
        backdate_run(&store, pending.id, 200).await;

        let running = store
            .create_run(new_run("deploy"))
            .await
            .unwrap()
            .into_run();
        store
            .update_run_status(running.id, RunStatus::Running)
            .await
            .unwrap();
        backdate_run(&store, running.id, 200).await;

        let policy = PurgePolicy {
            max_age_days: 90,
            max_runs_per_workflow: 10000,
            dry_run: false,
        };
        let purger = build(store.clone(), policy);
        purger.tick().await;

        assert!(store.get_run(pending.id).await.unwrap().is_some());
        assert!(store.get_run(running.id).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn run_stops_on_shutdown() {
        let store = Arc::new(InMemoryStore::new());
        let policy = PurgePolicy {
            max_age_days: 90,
            max_runs_per_workflow: 1000,
            dry_run: false,
        };
        let purger = build(store, policy);
        let shutdown = CancellationToken::new();
        shutdown.cancel();

        tokio::time::timeout(Duration::from_secs(5), purger.run(shutdown))
            .await
            .expect("purger stopped");
    }
}
