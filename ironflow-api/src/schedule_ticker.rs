//! Unified schedule executor for all workflow schedules.
//!
//! All schedules live in the database -- both those created via the REST API
//! and those declared by a [`WorkflowHandler::schedule()`]. At server startup,
//! call [`sync_handler_schedules`](crate::schedule_sync::sync_handler_schedules)
//! to reconcile handler-declared schedules, then spawn [`ScheduleTicker::run`]
//! which polls [`list_due_schedules`] and fires each due schedule with
//! [`fire_due_schedule`]: the runs of the occurrences it catches up and the
//! next trigger time are written in one transaction, or not at all.
//!
//! Which occurrences run follows the schedule's policy: occurrences missed
//! while the server was down are caught up (`latest`, `all` or `skip`) within
//! the catch-up window, and an `overlap = skip` schedule starts no run while
//! one of its runs is still active. Every dropped occurrence is logged,
//! counted in `ironflow_schedule_missed_total` and published as
//! [`Event::ScheduleOccurrencesMissed`] when the ticker has an engine.
//!
//! [`WorkflowHandler::schedule()`]: ironflow_engine::handler::WorkflowHandler::schedule
//! [`list_due_schedules`]: ironflow_store::schedule_store::ScheduleStore::list_due_schedules
//! [`fire_due_schedule`]: ironflow_store::schedule_store::ScheduleStore::fire_due_schedule

use std::sync::Arc;
use std::time::Duration;

use chrono::{TimeDelta, Utc};
use ironflow_engine::engine::Engine;
use ironflow_engine::notify::{Event, ScheduleOccurrencesMissedEvent};
use ironflow_store::entities::{Schedule, ScheduleMissReason, ScheduleNext};
use ironflow_store::store::Store;
use tokio::time::interval;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

#[cfg(feature = "prometheus")]
use ironflow_core::metric_names::{SCHEDULE_FIRE_ERRORS_TOTAL, SCHEDULE_MISSED_TOTAL};
#[cfg(feature = "prometheus")]
use metrics::counter;

use crate::schedule_clock::{MIN_ON_TIME_GRACE, MissedOccurrences, plan_firing};

/// Count a schedule that failed to fire or was disabled by an error.
fn record_fire_error(schedule_id: Uuid) {
    #[cfg(feature = "prometheus")]
    counter!(SCHEDULE_FIRE_ERRORS_TOTAL, "schedule" => schedule_id.to_string()).increment(1);
    #[cfg(not(feature = "prometheus"))]
    let _ = schedule_id;
}

/// Count occurrences a schedule did not run.
fn record_missed_metric(schedule_id: Uuid, count: u64) {
    #[cfg(feature = "prometheus")]
    counter!(SCHEDULE_MISSED_TOTAL, "schedule" => schedule_id.to_string()).increment(count);
    #[cfg(not(feature = "prometheus"))]
    let _ = (schedule_id, count);
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
/// let engine = Arc::new(Engine::new(store.clone(), Arc::new(ClaudeCodeProvider::new())));
/// sync_handler_schedules(&engine, store.as_ref()).await?;
///
/// let ticker = ScheduleTicker::new(store)
///     .engine(engine)
///     .interval(Duration::from_secs(10));
/// tokio::spawn(ticker.run(CancellationToken::new()));
/// # Ok(())
/// # }
/// ```
pub struct ScheduleTicker {
    store: Arc<dyn Store>,
    engine: Option<Arc<Engine>>,
    interval: Duration,
}

impl ScheduleTicker {
    /// Create a ticker with the default interval.
    pub fn new(store: Arc<dyn Store>) -> Self {
        Self {
            store,
            engine: None,
            interval: DEFAULT_TICK_INTERVAL,
        }
    }

    /// Publish [`Event::ScheduleOccurrencesMissed`] through this engine's
    /// event publisher whenever a schedule drops occurrences.
    ///
    /// Without an engine, missed occurrences are only logged and counted.
    pub fn engine(mut self, engine: Arc<Engine>) -> Self {
        self.engine = Some(engine);
        self
    }

    /// Set how often due schedules are polled.
    pub fn interval(mut self, interval: Duration) -> Self {
        self.interval = interval;
        self
    }

    /// How late an occurrence may fire and still count as on time: two tick
    /// intervals, and never less than [`MIN_ON_TIME_GRACE`].
    fn on_time_grace(&self) -> TimeDelta {
        let grace = MIN_ON_TIME_GRACE.max(self.interval.saturating_mul(2));
        TimeDelta::from_std(grace).unwrap_or(TimeDelta::MAX)
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
    /// The schedule's catch-up policy decides which of its missed occurrences
    /// run; all of them are fired atomically: a schedule whose firing fails
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

        let grace = self.on_time_grace();
        for schedule in due {
            let Some(occurrence) = schedule.next_trigger_at else {
                continue;
            };
            let firing = plan_firing(&schedule, occurrence, Utc::now(), grace);

            match self
                .store
                .fire_due_schedule(schedule.id, occurrence, firing.plan.clone())
                .await
            {
                Ok(Some(fired)) => {
                    for scheduled in &fired.runs {
                        info!(
                            schedule_id = %schedule.id,
                            workflow = %schedule.workflow_name,
                            run_id = %scheduled.run.run().id,
                            occurrence = %scheduled.occurrence,
                            replayed = !scheduled.run.is_created(),
                            "schedule fired"
                        );
                    }
                    // Traced only once the firing is committed: a firing that
                    // fails is retried, and one won by another instance is
                    // traced there.
                    let overlapped =
                        MissedOccurrences::group(ScheduleMissReason::Overlap, &fired.overlapped);
                    for missed in firing.missed.iter().chain(&overlapped) {
                        self.record_missed(&schedule, missed);
                    }
                    if firing.truncated {
                        warn!(
                            schedule_id = %schedule.id,
                            workflow = %schedule.workflow_name,
                            "too many missed occurrences to enumerate, the counts are lower bounds"
                        );
                    }
                    if let ScheduleNext::Disable { error } = &firing.plan.next {
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

    /// Log, count and publish occurrences a schedule did not run.
    fn record_missed(&self, schedule: &Schedule, missed: &MissedOccurrences) {
        warn!(
            schedule_id = %schedule.id,
            workflow = %schedule.workflow_name,
            reason = %missed.reason,
            count = missed.count,
            first = %missed.first,
            last = %missed.last,
            "schedule occurrences missed"
        );
        record_missed_metric(schedule.id, missed.count);
        if let Some(engine) = &self.engine {
            let event = ScheduleOccurrencesMissedEvent {
                schedule_id: schedule.id,
                workflow_name: schedule.workflow_name.clone(),
                reason: missed.reason,
                count: missed.count,
                first: missed.first,
                last: missed.last,
                at: Utc::now(),
            };
            engine
                .event_publisher()
                .publish(Event::ScheduleOccurrencesMissed(event));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::Duration as StdDuration;

    use chrono::{DateTime, Duration as ChronoDuration, DurationRound, Utc};
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::notify::{EventSubscriber, SubscriberFuture};
    use ironflow_store::entities::{
        CatchupPolicy, NewSchedule, OverlapPolicy, Run, RunFilter, Schedule, SchedulePolicy,
        ScheduleSource, ScheduleUpdate, TriggerKind,
    };
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::store::Store;
    #[cfg(feature = "prometheus")]
    use metrics::with_local_recorder;
    #[cfg(feature = "prometheus")]
    use metrics_exporter_prometheus::PrometheusBuilder;
    use serde_json::json;
    #[cfg(feature = "prometheus")]
    use tokio::runtime::Builder;
    use tokio::time::{sleep, timeout};
    use uuid::Uuid;

    use chrono_tz::Tz;

    use crate::schedule_clock::schedule_next;

    use super::*;

    async fn store_with_schedule(
        cron: &str,
        due: DateTime<Utc>,
        policy: SchedulePolicy,
    ) -> (Arc<dyn Store>, Schedule) {
        let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
        let schedule = store
            .create_schedule(NewSchedule {
                workflow_name: "deploy".to_string(),
                cron_expression: cron.to_string(),
                inputs: json!({}),
                source: ScheduleSource::Api,
                priority: 0,
                created_by_user_id: Some(Uuid::now_v7()),
                next_trigger_at: Some(due),
                policy,
            })
            .await
            .expect("create schedule");
        (store, schedule)
    }

    async fn store_with_due_schedule(cron: &str) -> (Arc<dyn Store>, Schedule) {
        let due = Utc::now() - ChronoDuration::seconds(10);
        store_with_schedule(cron, due, SchedulePolicy::default()).await
    }

    /// The top of the current hour, and an hourly schedule due four hours
    /// before it: five occurrences to catch up, the last one current.
    async fn store_with_hourly_backlog(policy: SchedulePolicy) -> (Arc<dyn Store>, Schedule) {
        let hour = Utc::now()
            .duration_trunc(ChronoDuration::hours(1))
            .expect("truncate to the hour");
        store_with_schedule("0 * * * *", hour - ChronoDuration::hours(4), policy).await
    }

    fn policy(catchup: CatchupPolicy, overlap: OverlapPolicy) -> SchedulePolicy {
        SchedulePolicy {
            catchup,
            overlap,
            ..SchedulePolicy::default()
        }
    }

    async fn all_runs(store: &Arc<dyn Store>) -> Vec<Run> {
        store
            .list_runs(RunFilter::default(), 1, 100)
            .await
            .expect("list runs")
            .items
    }

    /// Make the schedule due again, as a later occurrence would.
    async fn make_due_again(store: &Arc<dyn Store>, id: Uuid) {
        store
            .update_schedule(
                id,
                ScheduleUpdate {
                    next_trigger_at: Some(Some(Utc::now() - ChronoDuration::seconds(1))),
                    ..Default::default()
                },
            )
            .await
            .expect("make due");
    }

    /// Collects the events published during a test.
    struct EventCollector(Arc<Mutex<Vec<Event>>>);

    impl EventSubscriber for EventCollector {
        fn name(&self) -> &str {
            "event-collector"
        }

        fn handle<'a>(&'a self, event: &'a Event) -> SubscriberFuture<'a> {
            Box::pin(async move {
                self.0.lock().expect("collector lock").push(event.clone());
            })
        }
    }

    /// A ticker whose engine collects every missed-occurrence event.
    fn recording_ticker(store: Arc<dyn Store>) -> (ScheduleTicker, Arc<Mutex<Vec<Event>>>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut engine = Engine::new(store.clone(), Arc::new(ClaudeCodeProvider::new()));
        engine.subscribe(
            EventCollector(events.clone()),
            &[Event::SCHEDULE_OCCURRENCES_MISSED],
        );
        (ScheduleTicker::new(store).engine(Arc::new(engine)), events)
    }

    /// Wait until `count` events were delivered: subscribers run on spawned tasks.
    async fn collected(events: &Arc<Mutex<Vec<Event>>>, count: usize) -> Vec<Event> {
        timeout(StdDuration::from_secs(5), async {
            loop {
                let seen = events.lock().expect("collector lock").clone();
                if seen.len() >= count {
                    return seen;
                }
                sleep(StdDuration::from_millis(10)).await;
            }
        })
        .await
        .expect("events delivered")
    }

    fn missed_events(events: &[Event]) -> Vec<ScheduleOccurrencesMissedEvent> {
        events
            .iter()
            .filter_map(|event| match event {
                Event::ScheduleOccurrencesMissed(payload) => Some(payload.clone()),
                _ => None,
            })
            .collect()
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
        // A yearly cron: no later occurrence can fall between the due one and
        // the tick and supersede it.
        let (store, schedule) = store_with_due_schedule("0 0 1 1 *").await;
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
        match schedule_next("* * * * *", Tz::UTC) {
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
                policy: SchedulePolicy::default(),
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
                        policy: SchedulePolicy::default(),
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

    #[tokio::test]
    async fn tick_catchup_all_creates_one_run_per_missed_occurrence() {
        let (store, schedule) =
            store_with_hourly_backlog(policy(CatchupPolicy::All, OverlapPolicy::Allow)).await;
        let due = schedule.next_trigger_at.expect("due");

        ScheduleTicker::new(store.clone()).tick().await;

        let runs = all_runs(&store).await;
        assert_eq!(runs.len(), 5, "{runs:?}");
        let mut occurrences: Vec<DateTime<Utc>> = runs
            .iter()
            .map(|run| match &run.trigger {
                TriggerKind::Cron {
                    schedule_id,
                    scheduled_for,
                    ..
                } => {
                    assert_eq!(*schedule_id, Some(schedule.id));
                    scheduled_for.expect("a scheduled occurrence")
                }
                other => panic!("expected a cron trigger, got {other:?}"),
            })
            .collect();
        occurrences.sort();
        let expected: Vec<DateTime<Utc>> = (0..5)
            .map(|hours| due + ChronoDuration::hours(hours))
            .collect();
        assert_eq!(occurrences, expected);

        let s = reload(&store, schedule.id).await;
        assert!(s.next_trigger_at.expect("next trigger") > Utc::now());
    }

    #[tokio::test]
    async fn tick_catchup_latest_creates_a_single_run() {
        let (store, schedule) = store_with_hourly_backlog(SchedulePolicy::default()).await;
        let latest = schedule.next_trigger_at.expect("due") + ChronoDuration::hours(4);

        ScheduleTicker::new(store.clone()).tick().await;

        let runs = all_runs(&store).await;
        assert_eq!(runs.len(), 1, "{runs:?}");
        assert_eq!(
            runs[0].trigger,
            TriggerKind::Cron {
                schedule: "0 * * * *".to_string(),
                schedule_id: Some(schedule.id),
                scheduled_for: Some(latest),
            }
        );
    }

    #[tokio::test]
    async fn tick_catchup_publishes_missed_event() {
        let (store, schedule) = store_with_hourly_backlog(SchedulePolicy::default()).await;
        let due = schedule.next_trigger_at.expect("due");
        let (ticker, events) = recording_ticker(store);

        ticker.tick().await;

        let missed = missed_events(&collected(&events, 1).await);
        assert_eq!(missed.len(), 1, "{missed:?}");
        assert_eq!(missed[0].schedule_id, schedule.id);
        assert_eq!(missed[0].workflow_name, "deploy");
        assert_eq!(missed[0].reason, ScheduleMissReason::Superseded);
        assert_eq!(missed[0].count, 4);
        assert_eq!(missed[0].first, due);
        assert_eq!(missed[0].last, due + ChronoDuration::hours(3));
    }

    #[tokio::test]
    async fn tick_catchup_skip_drops_every_late_occurrence() {
        let skip = policy(CatchupPolicy::Skip, OverlapPolicy::Allow);
        let due = Utc::now() - ChronoDuration::minutes(30);
        // A yearly cron: the due occurrence is the only one, half an hour late.
        let (store, schedule) = store_with_schedule("0 0 1 1 *", due, skip).await;
        let (ticker, events) = recording_ticker(store.clone());

        ticker.tick().await;

        assert!(all_runs(&store).await.is_empty());
        let missed = missed_events(&collected(&events, 1).await);
        assert_eq!(missed[0].reason, ScheduleMissReason::CatchupSkip);
        assert_eq!(missed[0].count, 1);
        let s = reload(&store, schedule.id).await;
        assert!(s.last_triggered_at.is_none());
        assert!(s.next_trigger_at.expect("next trigger") > Utc::now());
    }

    #[tokio::test]
    async fn tick_overlap_skip_skips_while_a_run_is_active() {
        let skip = policy(CatchupPolicy::Latest, OverlapPolicy::Skip);
        let due = Utc::now() - ChronoDuration::seconds(10);
        let (store, schedule) = store_with_schedule("0 0 1 1 *", due, skip).await;
        let (ticker, events) = recording_ticker(store.clone());

        ticker.tick().await;
        assert_eq!(all_runs(&store).await.len(), 1);
        let first_trigger = reload(&store, schedule.id).await.last_triggered_at;

        make_due_again(&store, schedule.id).await;
        ticker.tick().await;

        let runs = all_runs(&store).await;
        assert_eq!(runs.len(), 1, "the active run blocks the next occurrence");
        let missed = missed_events(&collected(&events, 1).await);
        assert_eq!(missed.len(), 1, "{missed:?}");
        assert_eq!(missed[0].reason, ScheduleMissReason::Overlap);
        assert_eq!(missed[0].count, 1);
        let s = reload(&store, schedule.id).await;
        assert_eq!(s.last_triggered_at, first_trigger);
        assert!(s.next_trigger_at.expect("next trigger") > Utc::now());
    }

    #[tokio::test]
    async fn tick_overlap_allow_stacks_runs() {
        let due = Utc::now() - ChronoDuration::seconds(10);
        let (store, schedule) =
            store_with_schedule("0 0 1 1 *", due, SchedulePolicy::default()).await;
        let ticker = ScheduleTicker::new(store.clone());

        ticker.tick().await;
        make_due_again(&store, schedule.id).await;
        ticker.tick().await;

        assert_eq!(all_runs(&store).await.len(), 2);
    }

    #[cfg(feature = "prometheus")]
    #[test]
    fn tick_catchup_counts_missed_occurrences_per_schedule() {
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");

        let schedule_id = with_local_recorder(&recorder, || {
            runtime.block_on(async {
                let (store, schedule) = store_with_hourly_backlog(SchedulePolicy::default()).await;
                ScheduleTicker::new(store).tick().await;
                schedule.id
            })
        });

        let rendered = handle.render();
        assert!(
            rendered.contains(&format!(
                "ironflow_schedule_missed_total{{schedule=\"{schedule_id}\"}} 4"
            )),
            "{rendered}"
        );
    }
}
