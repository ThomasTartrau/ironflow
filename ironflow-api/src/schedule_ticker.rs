//! Unified schedule executor for all workflow schedules.
//!
//! All schedules live in the database -- both those created via the REST API
//! and those declared by a [`WorkflowHandler::schedule()`]. At server startup,
//! call [`sync_handler_schedules`](crate::schedule_sync::sync_handler_schedules)
//! to reconcile handler-declared schedules, then spawn [`ScheduleTicker::run`]
//! which polls [`list_due_schedules`] and fires each due occurrence with
//! [`fire_due_schedule`]: the run creation and the next trigger time are
//! written in one transaction, or not at all.
//!
//! [`list_due_schedules`]: ironflow_store::schedule_store::ScheduleStore::list_due_schedules
//! [`fire_due_schedule`]: ironflow_store::schedule_store::ScheduleStore::fire_due_schedule

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use croner::Cron;
use ironflow_store::entities::ScheduleNext;
use ironflow_store::store::Store;
use tokio::time::interval;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

#[cfg(feature = "prometheus")]
use ironflow_core::metric_names::SCHEDULE_FIRE_ERRORS_TOTAL;
#[cfg(feature = "prometheus")]
use metrics::counter;

/// Compute the next trigger time from a cron expression (5-field standard format).
pub(crate) fn next_trigger(cron_str: &str) -> Result<Option<DateTime<Utc>>, String> {
    let mut cron = Cron::new(cron_str);
    cron.pattern.with_seconds_optional = true;
    let cron = cron
        .parse()
        .map_err(|e| format!("invalid cron expression: {e}"))?;
    let next = cron
        .find_next_occurrence(&Utc::now(), false)
        .map_err(|e| format!("cannot compute next trigger: {e}"))?;
    Ok(Some(next))
}

/// What a schedule with this cron expression does after firing: fire again at
/// its next occurrence, or be disabled when that occurrence cannot be computed.
pub(crate) fn schedule_next(cron_str: &str) -> ScheduleNext {
    match next_trigger(cron_str) {
        Ok(Some(at)) => ScheduleNext::At(at),
        Ok(None) => ScheduleNext::Disable {
            error: "cannot compute next trigger: no next occurrence".to_string(),
        },
        Err(error) => ScheduleNext::Disable { error },
    }
}

/// Count a schedule that failed to fire or was disabled by an error.
fn record_fire_error(schedule_id: Uuid) {
    #[cfg(feature = "prometheus")]
    counter!(SCHEDULE_FIRE_ERRORS_TOTAL, "schedule" => schedule_id.to_string()).increment(1);
    #[cfg(not(feature = "prometheus"))]
    let _ = schedule_id;
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

    /// Fire every due schedule once.
    ///
    /// Each occurrence is fired atomically: a schedule whose firing fails
    /// stays due and is retried on the next tick. A schedule whose next
    /// occurrence cannot be computed fires its due run, then is disabled with
    /// the reason in `last_error`.
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
            let Some(occurrence) = schedule.next_trigger_at else {
                continue;
            };
            let next = schedule_next(&schedule.cron_expression);

            match self
                .store
                .fire_due_schedule(schedule.id, occurrence, next.clone())
                .await
            {
                Ok(Some(firing)) => {
                    info!(
                        schedule_id = %schedule.id,
                        workflow = %schedule.workflow_name,
                        run_id = %firing.run.run().id,
                        replayed = !firing.run.is_created(),
                        "schedule fired"
                    );
                    if let ScheduleNext::Disable { error } = next {
                        warn!(
                            schedule_id = %schedule.id,
                            workflow = %schedule.workflow_name,
                            error = %error,
                            "cannot compute next trigger, schedule disabled"
                        );
                        record_fire_error(schedule.id);
                    }
                }
                Ok(None) => {
                    debug!(
                        schedule_id = %schedule.id,
                        "schedule occurrence already fired or no longer due"
                    );
                }
                Err(err) => {
                    error!(
                        schedule_id = %schedule.id,
                        workflow = %schedule.workflow_name,
                        error = %err,
                        "failed to fire schedule, it stays due and is retried next tick"
                    );
                    record_fire_error(schedule.id);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration as ChronoDuration, Utc};
    use ironflow_store::entities::{
        NewSchedule, RunFilter, Schedule, ScheduleSource, ScheduleUpdate,
    };
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::store::Store;
    #[cfg(feature = "prometheus")]
    use metrics::with_local_recorder;
    #[cfg(feature = "prometheus")]
    use metrics_exporter_prometheus::PrometheusBuilder;
    use serde_json::json;
    use std::sync::Arc;
    #[cfg(feature = "prometheus")]
    use tokio::runtime::Builder;
    use uuid::Uuid;

    use super::*;

    async fn store_with_due_schedule(cron: &str) -> (Arc<dyn Store>, Schedule) {
        let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
        let schedule = store
            .create_schedule(NewSchedule {
                workflow_name: "deploy".to_string(),
                cron_expression: cron.to_string(),
                inputs: json!({}),
                source: ScheduleSource::Api,
                priority: 0,
                created_by_user_id: Some(Uuid::now_v7()),
                next_trigger_at: Some(Utc::now() - ChronoDuration::seconds(10)),
            })
            .await
            .expect("create schedule");
        (store, schedule)
    }

    async fn make_store_with_due_schedule() -> (Arc<dyn Store>, Uuid) {
        let (store, schedule) = store_with_due_schedule("* * * * *").await;
        (store, schedule.id)
    }

    async fn reload(store: &Arc<dyn Store>, id: Uuid) -> Schedule {
        store
            .find_schedule_by_id(id)
            .await
            .expect("find")
            .expect("exists")
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

        let updated = reload(&store, schedule_id).await;
        assert!(updated.last_triggered_at.is_some());
        assert!(updated.next_trigger_at.is_some());
        assert!(updated.next_trigger_at.unwrap() > Utc::now() - ChronoDuration::seconds(1));
    }

    #[tokio::test]
    async fn tick_run_carries_the_occurrence_key() {
        let (store, schedule) = store_with_due_schedule("* * * * *").await;
        let occurrence = schedule.next_trigger_at.expect("due");

        ScheduleTicker::new(store.clone()).tick().await;

        let runs = store
            .list_runs(RunFilter::default(), 1, 10)
            .await
            .expect("list runs");
        assert_eq!(runs.items.len(), 1);
        assert_eq!(
            runs.items[0].idempotency_key.as_deref(),
            Some(Schedule::occurrence_key(schedule.id, occurrence).as_str())
        );
    }

    /// #173: an instance that saw the schedule due and stopped before firing
    /// it used to leave it active with no next trigger, never fired again.
    #[tokio::test]
    async fn tick_fires_a_schedule_an_interrupted_tick_left_due() {
        let (store, schedule_id) = make_store_with_due_schedule().await;
        // A first instance lists the due schedules, then stops before firing.
        let seen = store.list_due_schedules().await.expect("list due");
        assert_eq!(seen.len(), 1);

        ScheduleTicker::new(store.clone()).tick().await;

        let s = reload(&store, schedule_id).await;
        assert!(
            !(s.is_active() && s.next_trigger_at.is_none()),
            "active schedule without next trigger: {s:?}"
        );
        let runs = store
            .list_runs(RunFilter::default(), 1, 10)
            .await
            .expect("list runs");
        assert_eq!(runs.items.len(), 1);
    }

    /// #173: a cron expression whose next occurrence cannot be computed used
    /// to leave the schedule active with no next trigger.
    #[tokio::test]
    async fn tick_disables_a_schedule_whose_next_trigger_cannot_be_computed() {
        // February 30th never happens.
        let (store, schedule) = store_with_due_schedule("0 0 30 2 *").await;

        ScheduleTicker::new(store.clone()).tick().await;

        let s = reload(&store, schedule.id).await;
        assert!(
            !(s.is_active() && s.next_trigger_at.is_none()),
            "active schedule without next trigger: {s:?}"
        );
        assert!(!s.is_active());
        let error = s.last_error.expect("disable reason stored");
        assert!(error.contains("cannot compute next trigger"), "{error}");

        // The due occurrence still ran.
        let runs = store
            .list_runs(RunFilter::default(), 1, 10)
            .await
            .expect("list runs");
        assert_eq!(runs.items.len(), 1);
    }

    #[tokio::test]
    async fn tick_disables_a_schedule_whose_cron_no_longer_parses() {
        let (store, schedule) = store_with_due_schedule("not-a-cron").await;

        ScheduleTicker::new(store.clone()).tick().await;

        let s = reload(&store, schedule.id).await;
        assert!(!s.is_active());
        assert!(s.next_trigger_at.is_none());
        let error = s.last_error.expect("disable reason stored");
        assert!(error.contains("invalid cron expression"), "{error}");
    }

    #[test]
    fn schedule_next_for_a_valid_cron_is_in_the_future() {
        match schedule_next("* * * * *") {
            ScheduleNext::At(at) => assert!(at > Utc::now()),
            ScheduleNext::Disable { error } => panic!("unexpected disable: {error}"),
        }
    }

    #[tokio::test]
    async fn tick_skips_disabled_schedule() {
        let (store, schedule_id) = make_store_with_due_schedule().await;

        store
            .update_schedule(
                schedule_id,
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
                priority: 0,
                created_by_user_id: Some(Uuid::now_v7()),
                next_trigger_at: Some(Utc::now() + ChronoDuration::hours(1)),
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

    #[cfg(feature = "prometheus")]
    #[test]
    fn tick_counts_fire_errors_per_schedule() {
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");

        let (broken, healthy) = with_local_recorder(&recorder, || {
            runtime.block_on(async {
                let (store, broken) = store_with_due_schedule("0 0 30 2 *").await;
                let healthy = store
                    .create_schedule(NewSchedule {
                        workflow_name: "deploy".to_string(),
                        cron_expression: "* * * * *".to_string(),
                        inputs: json!({}),
                        source: ScheduleSource::Api,
                        priority: 0,
                        created_by_user_id: None,
                        next_trigger_at: Some(Utc::now() - ChronoDuration::seconds(10)),
                    })
                    .await
                    .expect("create healthy");
                ScheduleTicker::new(store).tick().await;
                (broken.id, healthy.id)
            })
        });

        let rendered = handle.render();
        assert!(
            rendered.contains(&format!(
                "ironflow_schedule_fire_errors_total{{schedule=\"{broken}\"}} 1"
            )),
            "{rendered}"
        );
        assert!(!rendered.contains(&healthy.to_string()), "{rendered}");
    }
}
