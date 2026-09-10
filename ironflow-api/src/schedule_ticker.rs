//! Unified schedule executor for all workflow schedules.
//!
//! All schedules live in the database -- both those created via the REST API
//! and those declared by a [`WorkflowHandler::schedule()`]. At server startup,
//! call [`sync_handler_schedules`] to seed DB rows for handler-declared
//! schedules, then spawn [`ScheduleTicker::run`] which polls
//! [`list_due_schedules`] and creates a run for each schedule whose
//! `next_trigger_at` has passed.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use croner::Cron;
use ironflow_engine::engine::Engine;
use ironflow_store::entities::{NewRun, NewSchedule, RunActor, Schedule, ScheduleUpdate, TriggerKind};
use ironflow_store::store::Store;
use tokio::time::interval;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
use uuid::Uuid;

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
pub(crate) fn new_run_from_schedule(
    schedule: &Schedule,
    created_by: Option<RunActor>,
) -> NewRun {
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

/// Seed a DB schedule for each handler that declares [`WorkflowHandler::schedule()`].
///
/// Existing schedules (matched by `workflow_name`) are left untouched. Only
/// missing ones are created. Call this once at server startup, before
/// spawning the [`ScheduleTicker`].
///
/// # Errors
///
/// Returns the first store error encountered. Schedules created before the
/// error are kept.
pub async fn sync_handler_schedules(
    engine: &Engine,
    store: &dyn Store,
) -> Result<(), ironflow_store::error::StoreError> {
    let handlers = engine.scheduled_handlers();
    if handlers.is_empty() {
        return Ok(());
    }

    let existing = store.list_schedules(1, 1000).await?;
    let existing_names: std::collections::HashSet<&str> = existing
        .items
        .iter()
        .map(|s| s.workflow_name.as_str())
        .collect();

    for (name, cron_schedule) in handlers {
        if existing_names.contains(name) {
            continue;
        }

        let next = next_trigger(cron_schedule.as_str()).unwrap_or(None);

        store
            .create_schedule(NewSchedule {
                workflow_name: name.to_string(),
                cron_expression: cron_schedule.as_str().to_string(),
                inputs: serde_json::json!({}),
                created_by_user_id: Uuid::nil(),
                next_trigger_at: next,
            })
            .await?;

        info!(
            workflow = %name,
            schedule = %cron_schedule,
            "synced handler-declared schedule to DB"
        );
    }

    Ok(())
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
/// use ironflow_api::schedule_ticker::{ScheduleTicker, sync_handler_schedules};
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
        let due = match self.store.list_due_schedules().await {
            Ok(due) => due,
            Err(err) => {
                error!(error = %err, "failed to list due schedules");
                return;
            }
        };

        if due.is_empty() {
            return;
        }

        info!(count = due.len(), "firing due schedules");

        for schedule in due {
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
                        "cannot compute next trigger, disabling schedule"
                    );
                    None
                }
            };

            let update = ScheduleUpdate {
                last_triggered_at: Some(Some(Utc::now())),
                next_trigger_at: Some(next),
                ..Default::default()
            };

            if let Err(err) = self.store.update_schedule(schedule.id, update).await {
                error!(
                    schedule_id = %schedule.id,
                    error = %err,
                    "failed to update schedule after trigger"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use ironflow_store::entities::{NewSchedule, RunFilter};
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
                created_by_user_id: Uuid::now_v7(),
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
                created_by_user_id: Uuid::now_v7(),
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
                created_by_user_id: Uuid::now_v7(),
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
    async fn sync_creates_missing_handler_schedules() {
        use ironflow_core::providers::claude::ClaudeCodeProvider;
        use ironflow_engine::context::WorkflowContext;
        use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
        use ironflow_engine::prelude::CronSchedule;

        struct Scheduled {
            cron: CronSchedule,
        }
        impl WorkflowHandler for Scheduled {
            fn name(&self) -> &str {
                "nightly-cleanup"
            }
            fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
                Box::pin(async { Ok(()) })
            }
            fn schedule(&self) -> Option<&CronSchedule> {
                Some(&self.cron)
            }
        }

        let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
        let provider = Arc::new(ClaudeCodeProvider::new());
        let mut engine = ironflow_engine::engine::Engine::new(store.clone(), provider);
        engine
            .register(Scheduled {
                cron: CronSchedule::new("0 0 * * *").unwrap(),
            })
            .expect("register");

        sync_handler_schedules(&engine, store.as_ref())
            .await
            .expect("sync");

        let page = store.list_schedules(1, 10).await.expect("list");
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].workflow_name, "nightly-cleanup");
        assert_eq!(page.items[0].cron_expression, "0 0 * * *");
        assert!(page.items[0].next_trigger_at.is_some());
    }

    #[tokio::test]
    async fn sync_skips_existing_schedule() {
        use ironflow_core::providers::claude::ClaudeCodeProvider;
        use ironflow_engine::context::WorkflowContext;
        use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
        use ironflow_engine::prelude::CronSchedule;

        struct Scheduled {
            cron: CronSchedule,
        }
        impl WorkflowHandler for Scheduled {
            fn name(&self) -> &str {
                "deploy"
            }
            fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
                Box::pin(async { Ok(()) })
            }
            fn schedule(&self) -> Option<&CronSchedule> {
                Some(&self.cron)
            }
        }

        let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
        store
            .create_schedule(NewSchedule {
                workflow_name: "deploy".to_string(),
                cron_expression: "*/5 * * * *".to_string(),
                inputs: json!({"env": "prod"}),
                created_by_user_id: Uuid::now_v7(),
                next_trigger_at: Some(Utc::now()),
            })
            .await
            .expect("seed");

        let provider = Arc::new(ClaudeCodeProvider::new());
        let mut engine = ironflow_engine::engine::Engine::new(store.clone(), provider);
        engine
            .register(Scheduled {
                cron: CronSchedule::new("0 0 * * *").unwrap(),
            })
            .expect("register");

        sync_handler_schedules(&engine, store.as_ref())
            .await
            .expect("sync");

        let page = store.list_schedules(1, 10).await.expect("list");
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].cron_expression, "*/5 * * * *");
    }
}
