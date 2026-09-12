//! Unified schedule executor for all workflow schedules.
//!
//! All schedules live in the database -- both those created via the REST API
//! and those declared by a [`WorkflowHandler::schedule()`]. At server startup,
//! call [`sync_handler_schedules`](crate::schedule_sync::sync_handler_schedules)
//! to reconcile handler-declared schedules, then spawn [`ScheduleTicker::run`]
//! which polls [`claim_due_schedules`] and creates a run for each claimed
//! schedule.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use croner::Cron;
use ironflow_store::entities::{NewRun, RunActor, Schedule, ScheduleUpdate, TriggerKind};
use ironflow_store::store::Store;
use tokio::time::interval;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

/// Compute the next trigger time from a cron expression (5-field standard format).
pub(crate) fn next_trigger(
    cron_str: &str,
) -> Result<Option<chrono::DateTime<chrono::Utc>>, String> {
    let mut cron = Cron::new(cron_str);
    cron.pattern.with_seconds_optional = true;
    let cron = cron
        .parse()
        .map_err(|e| format!("invalid cron expression: {e}"))?;
    let next = cron
        .find_next_occurrence(&chrono::Utc::now(), false)
        .map_err(|e| format!("cannot compute next trigger: {e}"))?;
    Ok(Some(next))
}

/// Build a [`NewRun`] from a schedule's fields.
pub(crate) fn new_run_from_schedule(schedule: &Schedule, created_by: Option<RunActor>) -> NewRun {
    NewRun {
        workflow_name: schedule.workflow_name.clone(),
        trigger: TriggerKind::Cron {
            schedule: schedule.cron_expression.clone(),
        },
        payload: schedule.inputs.clone(),
        max_retries: 0,
        handler_version: None,
        labels: HashMap::new(),
        scheduled_at: None,
        created_by,
        idempotency_key: None,
        max_cost_usd: None,
    }
}

/// How often the ticker checks for due schedules.
pub const DEFAULT_TICK_INTERVAL: Duration = Duration::from_secs(15);

/// Periodic task that fires DB-persisted schedules.
///
/// # Examples
///
/// ```no_run
/// use std::sync::Arc;
/// use std::time::Duration;
/// use ironflow_api::schedule_ticker::ScheduleTicker;
/// use ironflow_api::schedule_sync::sync_handler_schedules;
/// use ironflow_engine::engine::Engine;
/// use ironflow_core::providers::claude::ClaudeCodeProvider;
/// use ironflow_store::memory::InMemoryStore;
/// use ironflow_store::store::Store;
/// use tokio_util::sync::CancellationToken;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
/// let engine = Engine::new(store.clone(), Arc::new(ClaudeCodeProvider::new()));
/// sync_handler_schedules(&engine, store.as_ref()).await?;
///
/// let ticker = ScheduleTicker::new(store).interval(Duration::from_secs(10));
/// tokio::spawn(ticker.run(CancellationToken::new()));
/// # Ok(())
/// # }
/// ```
pub struct ScheduleTicker {
    store: Arc<dyn Store>,
    interval: Duration,
}

impl ScheduleTicker {
    /// Create a ticker with the default interval.
    pub fn new(store: Arc<dyn Store>) -> Self {
        Self {
            store,
            interval: DEFAULT_TICK_INTERVAL,
        }
    }

    /// Set how often due schedules are polled.
    pub fn interval(mut self, interval: Duration) -> Self {
        self.interval = interval;
        self
    }

    /// Run the tick loop until `shutdown` is cancelled.
    ///
    /// Store errors are logged per-schedule; the loop keeps going.
    pub async fn run(self, shutdown: CancellationToken) {
        let mut ticker = interval(self.interval);
        ticker.tick().await;

        info!(
            interval_secs = self.interval.as_secs(),
            "schedule ticker started"
        );

        loop {
            tokio::select! {
                _ = shutdown.cancelled() => {
                    info!("schedule ticker stopped");
                    return;
                }
                _ = ticker.tick() => {
                    self.tick().await;
                }
            }
        }
    }

    /// Process one batch of due schedules.
    ///
    /// Exposed for tests and for callers that drive the tick themselves.
    pub async fn tick(&self) {
        let claimed = match self.store.claim_due_schedules().await {
            Ok(claimed) => claimed,
            Err(err) => {
                error!(error = %err, "failed to claim due schedules");
                return;
            }
        };

        if claimed.is_empty() {
            return;
        }

        info!(count = claimed.len(), "firing due schedules");

        for schedule in claimed {
            let run_result = self
                .store
                .create_run(new_run_from_schedule(&schedule, None))
                .await;

            match run_result {
                Ok(creation) => {
                    let run = creation.into_run();
                    info!(
                        schedule_id = %schedule.id,
                        workflow = %schedule.workflow_name,
                        run_id = %run.id,
                        "schedule fired"
                    );
                }
                Err(err) => {
                    error!(
                        schedule_id = %schedule.id,
                        workflow = %schedule.workflow_name,
                        error = %err,
                        "failed to create run for schedule"
                    );
                    continue;
                }
            }

            let next = match next_trigger(&schedule.cron_expression) {
                Ok(next) => next,
                Err(err) => {
                    warn!(
                        schedule_id = %schedule.id,
                        error = %err,
                        "cannot compute next trigger, schedule will remain paused"
                    );
                    None
                }
            };

            if let Err(err) = self
                .store
                .update_schedule(
                    schedule.id,
                    ScheduleUpdate {
                        next_trigger_at: Some(next),
                        ..Default::default()
                    },
                )
                .await
            {
                error!(
                    schedule_id = %schedule.id,
                    error = %err,
                    "failed to set next trigger time after firing"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use ironflow_store::entities::{NewSchedule, RunFilter, ScheduleSource};
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::store::Store;
    use serde_json::json;
    use std::sync::Arc;
    use uuid::Uuid;

    use super::*;

    async fn make_store_with_due_schedule() -> (Arc<dyn Store>, Uuid) {
        let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
        let schedule = store
            .create_schedule(NewSchedule {
                workflow_name: "deploy".to_string(),
                cron_expression: "* * * * *".to_string(),
                inputs: json!({}),
                source: ScheduleSource::Api,
                created_by_user_id: Some(Uuid::now_v7()),
                next_trigger_at: Some(Utc::now() - chrono::Duration::seconds(10)),
            })
            .await
            .expect("create schedule");
        (store, schedule.id)
    }

    #[tokio::test]
    async fn tick_creates_run_for_due_schedule() {
        let (store, schedule_id) = make_store_with_due_schedule().await;
        let ticker = ScheduleTicker::new(store.clone());

        ticker.tick().await;

        let runs = store
            .list_runs(RunFilter::default(), 1, 10)
            .await
            .expect("list runs");
        assert_eq!(runs.items.len(), 1);
        assert_eq!(runs.items[0].workflow_name, "deploy");

        let updated = store
            .find_schedule_by_id(schedule_id)
            .await
            .expect("find")
            .expect("exists");
        assert!(updated.last_triggered_at.is_some());
        assert!(updated.next_trigger_at.is_some());
        assert!(updated.next_trigger_at.unwrap() > Utc::now() - chrono::Duration::seconds(1));
    }

    #[tokio::test]
    async fn tick_skips_disabled_schedule() {
        let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
        let schedule = store
            .create_schedule(NewSchedule {
                workflow_name: "deploy".to_string(),
                cron_expression: "* * * * *".to_string(),
                inputs: json!({}),
                source: ScheduleSource::Api,
                created_by_user_id: Some(Uuid::now_v7()),
                next_trigger_at: Some(Utc::now() - chrono::Duration::seconds(10)),
            })
            .await
            .expect("create");

        store
            .update_schedule(
                schedule.id,
                ScheduleUpdate {
                    disabled_at: Some(Some(Utc::now())),
                    ..Default::default()
                },
            )
            .await
            .expect("disable");

        let ticker = ScheduleTicker::new(store.clone());
        ticker.tick().await;

        let runs = store
            .list_runs(RunFilter::default(), 1, 10)
            .await
            .expect("list runs");
        assert!(runs.items.is_empty());
    }

    #[tokio::test]
    async fn tick_skips_future_schedule() {
        let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
        store
            .create_schedule(NewSchedule {
                workflow_name: "deploy".to_string(),
                cron_expression: "* * * * *".to_string(),
                inputs: json!({}),
                source: ScheduleSource::Api,
                created_by_user_id: Some(Uuid::now_v7()),
                next_trigger_at: Some(Utc::now() + chrono::Duration::hours(1)),
            })
            .await
            .expect("create");

        let ticker = ScheduleTicker::new(store.clone());
        ticker.tick().await;

        let runs = store
            .list_runs(RunFilter::default(), 1, 10)
            .await
            .expect("list runs");
        assert!(runs.items.is_empty());
    }

    #[tokio::test]
    async fn tick_does_nothing_when_empty() {
        let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
        let ticker = ScheduleTicker::new(store.clone());
        ticker.tick().await;

        let runs = store
            .list_runs(RunFilter::default(), 1, 10)
            .await
            .expect("list runs");
        assert!(runs.items.is_empty());
    }

    #[tokio::test]
    async fn tick_does_not_double_fire() {
        let (store, _) = make_store_with_due_schedule().await;
        let ticker = ScheduleTicker::new(store.clone());

        ticker.tick().await;
        ticker.tick().await;

        let runs = store
            .list_runs(RunFilter::default(), 1, 10)
            .await
            .expect("list runs");
        assert_eq!(
            runs.items.len(),
            1,
            "second tick must not create a duplicate run"
        );
    }
}
