//! Reconciliation of handler-declared schedules with the database.
//!
//! Call [`sync_handler_schedules`] then [`repair_unscheduled_schedules`] once
//! at server startup, before spawning the
//! [`ScheduleTicker`](super::schedule_ticker::ScheduleTicker).

use std::collections::{HashMap, HashSet};

use chrono::Utc;
use ironflow_engine::engine::Engine;
use ironflow_store::entities::{
    MAX_PRIORITY, MIN_PRIORITY, NewSchedule, Schedule, ScheduleNext, ScheduleSource, ScheduleUpdate,
};
use ironflow_store::error::StoreError;
use ironflow_store::store::Store;
use serde_json::json;
use tracing::{info, warn};

use crate::schedule_ticker::schedule_next;

/// Reconcile DB schedules with handler-declared schedules.
///
/// Performs a three-way sync at startup:
/// 1. **Create** DB rows for handlers that declare a schedule but have no
///    corresponding `source = handler` row.
/// 2. **Update** the cron expression when the handler's cron changed, and the
///    priority when the handler's
///    [`priority`](ironflow_engine::handler::WorkflowHandler::priority) changed.
/// 3. **Delete** orphan `source = handler` rows whose handler was removed
///    from the code.
///
/// # Errors
///
/// Returns the first store error encountered. Changes applied before the
/// error are kept.
///
/// # Examples
///
/// ```no_run
/// use std::sync::Arc;
/// use ironflow_api::schedule_sync::sync_handler_schedules;
/// use ironflow_engine::engine::Engine;
/// use ironflow_core::providers::claude::ClaudeCodeProvider;
/// use ironflow_store::memory::InMemoryStore;
/// use ironflow_store::store::Store;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
/// let engine = Engine::new(store.clone(), Arc::new(ClaudeCodeProvider::new()));
/// sync_handler_schedules(&engine, store.as_ref()).await?;
/// # Ok(())
/// # }
/// ```
pub async fn sync_handler_schedules(engine: &Engine, store: &dyn Store) -> Result<(), StoreError> {
    let handlers = engine.scheduled_handlers();
    // The priority every run of the schedule gets, clamped like a run the
    // handler creates itself.
    let handler_map: HashMap<&str, (&str, i16)> = handlers
        .iter()
        .map(|(name, cron)| {
            let priority = engine.get_handler(name).map_or(0, |handler| {
                handler.priority().clamp(MIN_PRIORITY, MAX_PRIORITY)
            });
            (*name, (cron.as_str(), priority))
        })
        .collect();
    let handler_names: HashSet<&str> = handler_map.keys().copied().collect();

    let existing = store.list_schedules(1, 1000).await?;

    let handler_schedules: Vec<_> = existing
        .items
        .iter()
        .filter(|s| s.source == ScheduleSource::Handler)
        .collect();

    // 1. Delete orphans: handler-declared schedules whose handler no longer exists.
    for schedule in &handler_schedules {
        if !handler_names.contains(schedule.workflow_name.as_str()) {
            store.delete_schedule(schedule.id).await?;
            info!(
                workflow = %schedule.workflow_name,
                "removed orphan handler schedule (handler no longer exists)"
            );
        }
    }

    // Build lookup of remaining handler schedules by workflow name.
    let existing_map: HashMap<&str, &Schedule> = handler_schedules
        .iter()
        .filter(|s| handler_names.contains(s.workflow_name.as_str()))
        .map(|s| (s.workflow_name.as_str(), *s))
        .collect();

    for (name, (cron_str, priority)) in &handler_map {
        match existing_map.get(name) {
            Some(existing) => {
                let cron_changed = existing.cron_expression != *cron_str;
                let priority_changed = existing.priority != *priority;
                if !cron_changed && !priority_changed {
                    continue;
                }
                let mut update = ScheduleUpdate::default();
                if cron_changed {
                    // 2. Cron changed in code: update DB row. A schedule Ironflow
                    // disabled on an error is re-enabled by a cron that works.
                    let next = schedule_next(cron_str);
                    let reenable =
                        existing.last_error.is_some() && matches!(next, ScheduleNext::At(_));
                    update = next_trigger_update(next);
                    update.cron_expression = Some(cron_str.to_string());
                    if reenable {
                        update.disabled_at = Some(None);
                        update.last_error = Some(None);
                    }
                }
                if priority_changed {
                    update.priority = Some(*priority);
                }
                store.update_schedule(existing.id, update).await?;
                info!(
                    workflow = %name,
                    old_cron = %existing.cron_expression,
                    new_cron = %cron_str,
                    old_priority = existing.priority,
                    new_priority = *priority,
                    "updated handler schedule"
                );
            }
            None => {
                // 3. Missing: create a new handler schedule.
                let next = schedule_next(cron_str);
                let next_trigger_at = match next {
                    ScheduleNext::At(at) => Some(at),
                    ScheduleNext::Disable { .. } => None,
                };
                let created = store
                    .create_schedule(NewSchedule {
                        workflow_name: name.to_string(),
                        cron_expression: cron_str.to_string(),
                        inputs: json!({}),
                        source: ScheduleSource::Handler,
                        priority: *priority,
                        created_by_user_id: None,
                        next_trigger_at,
                    })
                    .await?;
                if let ScheduleNext::Disable { error } = next {
                    warn!(
                        workflow = %name,
                        schedule = %cron_str,
                        error = %error,
                        "cannot compute next trigger, handler schedule disabled"
                    );
                    store
                        .update_schedule(
                            created.id,
                            next_trigger_update(ScheduleNext::Disable { error }),
                        )
                        .await?;
                }
                info!(
                    workflow = %name,
                    schedule = %cron_str,
                    "synced handler-declared schedule to DB"
                );
            }
        }
    }

    Ok(())
}

/// The update that applies `next` to a schedule: its next trigger time, or
/// disabled with the reason in `last_error`.
fn next_trigger_update(next: ScheduleNext) -> ScheduleUpdate {
    match next {
        ScheduleNext::At(at) => ScheduleUpdate {
            next_trigger_at: Some(Some(at)),
            ..Default::default()
        },
        ScheduleNext::Disable { error } => ScheduleUpdate {
            disabled_at: Some(Some(Utc::now())),
            next_trigger_at: Some(None),
            last_error: Some(Some(error)),
            ..Default::default()
        },
    }
}

/// How many schedules [`repair_unscheduled_schedules`] reads per page.
const REPAIR_PAGE_SIZE: u32 = 100;

/// Repair active schedules that have no next trigger time.
///
/// Such a schedule never fires again. Earlier releases left schedules in that
/// state when a run creation failed, when the server stopped between the
/// claim and the update, or when the next trigger could not be computed.
/// Each one receives its next trigger time, or is disabled with the reason in
/// `last_error` when that time cannot be computed. Logs a `warn` per repaired
/// schedule and returns how many were repaired.
///
/// Call it once at server startup, after
/// [`sync_handler_schedules`] and before spawning the
/// [`ScheduleTicker`](super::schedule_ticker::ScheduleTicker).
///
/// # Errors
///
/// Returns the first store error encountered. Repairs applied before the
/// error are kept.
///
/// # Examples
///
/// ```no_run
/// use ironflow_api::schedule_sync::repair_unscheduled_schedules;
/// use ironflow_store::memory::InMemoryStore;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let store = InMemoryStore::new();
/// let repaired = repair_unscheduled_schedules(&store).await?;
/// println!("{repaired} schedules repaired");
/// # Ok(())
/// # }
/// ```
pub async fn repair_unscheduled_schedules(store: &dyn Store) -> Result<usize, StoreError> {
    let mut broken: Vec<Schedule> = Vec::new();
    let mut page = 1;
    loop {
        let batch = store.list_schedules(page, REPAIR_PAGE_SIZE).await?;
        let read = (page as u64) * (REPAIR_PAGE_SIZE as u64);
        let last = batch.items.is_empty() || read >= batch.total;
        broken.extend(
            batch
                .items
                .into_iter()
                .filter(|s| s.is_active() && s.next_trigger_at.is_none()),
        );
        if last {
            break;
        }
        page += 1;
    }

    for schedule in &broken {
        let next = schedule_next(&schedule.cron_expression);
        match &next {
            ScheduleNext::At(at) => warn!(
                schedule_id = %schedule.id,
                workflow = %schedule.workflow_name,
                next_trigger_at = %at,
                "repaired active schedule without next trigger"
            ),
            ScheduleNext::Disable { error } => warn!(
                schedule_id = %schedule.id,
                workflow = %schedule.workflow_name,
                error = %error,
                "disabled active schedule without next trigger: cannot compute it"
            ),
        }
        store
            .update_schedule(schedule.id, next_trigger_update(next))
            .await?;
    }

    Ok(broken.len())
}

#[cfg(test)]
mod tests {
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::context::WorkflowContext;
    use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
    use ironflow_engine::prelude::CronSchedule;
    use ironflow_store::memory::InMemoryStore;
    use std::sync::Arc;
    use uuid::Uuid;

    use super::*;

    struct NamedScheduled {
        wf_name: &'static str,
        cron: CronSchedule,
    }
    impl WorkflowHandler for NamedScheduled {
        fn name(&self) -> &str {
            self.wf_name
        }
        fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
            Box::pin(async { Ok(()) })
        }
        fn schedule(&self) -> Option<&CronSchedule> {
            Some(&self.cron)
        }
    }

    #[tokio::test]
    async fn creates_missing_handler_schedules() {
        let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
        let provider = Arc::new(ClaudeCodeProvider::new());
        let mut engine = ironflow_engine::engine::Engine::new(store.clone(), provider);
        engine
            .register(NamedScheduled {
                wf_name: "nightly-cleanup",
                cron: CronSchedule::new("0 0 * * *").unwrap(),
            })
            .expect("register");

        sync_handler_schedules(&engine, store.as_ref())
            .await
            .expect("sync");

        let page = store.list_schedules(1, 10).await.expect("list");
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].workflow_name, "nightly-cleanup");
        assert_eq!(page.items[0].source, ScheduleSource::Handler);
    }

    #[tokio::test]
    async fn skips_existing_handler_schedule() {
        let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
        store
            .create_schedule(NewSchedule {
                workflow_name: "deploy".to_string(),
                cron_expression: "*/5 * * * *".to_string(),
                inputs: json!({"env": "prod"}),
                source: ScheduleSource::Handler,
                priority: 0,
                created_by_user_id: None,
                next_trigger_at: None,
            })
            .await
            .expect("seed");

        let provider = Arc::new(ClaudeCodeProvider::new());
        let mut engine = ironflow_engine::engine::Engine::new(store.clone(), provider);
        engine
            .register(NamedScheduled {
                wf_name: "deploy",
                cron: CronSchedule::new("*/5 * * * *").unwrap(),
            })
            .expect("register");

        sync_handler_schedules(&engine, store.as_ref())
            .await
            .expect("sync");

        let page = store.list_schedules(1, 10).await.expect("list");
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].cron_expression, "*/5 * * * *");
    }

    #[tokio::test]
    async fn removes_orphan_handler_schedules() {
        let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
        store
            .create_schedule(NewSchedule {
                workflow_name: "removed-workflow".to_string(),
                cron_expression: "0 0 * * *".to_string(),
                inputs: json!({}),
                source: ScheduleSource::Handler,
                priority: 0,
                created_by_user_id: None,
                next_trigger_at: None,
            })
            .await
            .expect("seed orphan");

        // API schedule must NOT be removed.
        store
            .create_schedule(NewSchedule {
                workflow_name: "user-created".to_string(),
                cron_expression: "0 12 * * *".to_string(),
                inputs: json!({}),
                source: ScheduleSource::Api,
                priority: 0,
                created_by_user_id: Some(Uuid::now_v7()),
                next_trigger_at: None,
            })
            .await
            .expect("seed api");

        let provider = Arc::new(ClaudeCodeProvider::new());
        let engine = ironflow_engine::engine::Engine::new(store.clone(), provider);

        sync_handler_schedules(&engine, store.as_ref())
            .await
            .expect("sync");

        let page = store.list_schedules(1, 10).await.expect("list");
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].workflow_name, "user-created");
        assert_eq!(page.items[0].source, ScheduleSource::Api);
    }

    #[tokio::test]
    async fn updates_changed_cron() {
        let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
        store
            .create_schedule(NewSchedule {
                workflow_name: "deploy".to_string(),
                cron_expression: "0 0 * * *".to_string(),
                inputs: json!({}),
                source: ScheduleSource::Handler,
                priority: 0,
                created_by_user_id: None,
                next_trigger_at: None,
            })
            .await
            .expect("seed");

        let provider = Arc::new(ClaudeCodeProvider::new());
        let mut engine = ironflow_engine::engine::Engine::new(store.clone(), provider);
        engine
            .register(NamedScheduled {
                wf_name: "deploy",
                cron: CronSchedule::new("0 */6 * * *").unwrap(),
            })
            .expect("register");

        sync_handler_schedules(&engine, store.as_ref())
            .await
            .expect("sync");

        let page = store.list_schedules(1, 10).await.expect("list");
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].cron_expression, "0 */6 * * *");
    }

    fn engine_with(store: &Arc<dyn Store>, wf_name: &'static str, cron: &str) -> Engine {
        let provider = Arc::new(ClaudeCodeProvider::new());
        let mut engine = Engine::new(store.clone(), provider);
        engine
            .register(NamedScheduled {
                wf_name,
                cron: CronSchedule::new(cron).unwrap(),
            })
            .expect("register");
        engine
    }

    async fn seed(store: &Arc<dyn Store>, workflow: &str, cron: &str) -> Schedule {
        store
            .create_schedule(NewSchedule {
                workflow_name: workflow.to_string(),
                cron_expression: cron.to_string(),
                inputs: json!({}),
                source: ScheduleSource::Api,
                priority: 0,
                created_by_user_id: None,
                next_trigger_at: None,
            })
            .await
            .expect("seed")
    }

    async fn reload(store: &Arc<dyn Store>, id: Uuid) -> Schedule {
        store
            .find_schedule_by_id(id)
            .await
            .expect("find")
            .expect("exists")
    }

    /// #173: a handler cron with no computable next occurrence used to be
    /// synced as an active schedule without next trigger, never fired.
    #[tokio::test]
    async fn sync_disables_a_handler_schedule_whose_next_trigger_cannot_be_computed() {
        let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
        // February 30th never happens.
        let engine = engine_with(&store, "leap", "0 0 30 2 *");

        sync_handler_schedules(&engine, store.as_ref())
            .await
            .expect("sync");

        let page = store.list_schedules(1, 10).await.expect("list");
        assert_eq!(page.items.len(), 1);
        let s = &page.items[0];
        assert!(
            !s.is_active(),
            "active schedule without next trigger: {s:?}"
        );
        assert!(s.next_trigger_at.is_none());
        let error = s.last_error.as_deref().expect("disable reason stored");
        assert!(error.contains("cannot compute next trigger"), "{error}");
    }

    #[tokio::test]
    async fn sync_reenables_a_handler_schedule_disabled_on_error_once_its_cron_works() {
        let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
        sync_handler_schedules(&engine_with(&store, "leap", "0 0 30 2 *"), store.as_ref())
            .await
            .expect("first sync");

        sync_handler_schedules(&engine_with(&store, "leap", "0 0 1 3 *"), store.as_ref())
            .await
            .expect("second sync");

        let page = store.list_schedules(1, 10).await.expect("list");
        let s = &page.items[0];
        assert!(s.is_active());
        assert!(s.next_trigger_at.expect("next trigger") > Utc::now());
        assert!(s.last_error.is_none());
    }

    #[tokio::test]
    async fn sync_keeps_a_user_paused_handler_schedule_paused_on_cron_change() {
        let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
        sync_handler_schedules(&engine_with(&store, "deploy", "0 0 * * *"), store.as_ref())
            .await
            .expect("first sync");
        let id = store.list_schedules(1, 10).await.expect("list").items[0].id;
        store
            .update_schedule(
                id,
                ScheduleUpdate {
                    disabled_at: Some(Some(Utc::now())),
                    ..Default::default()
                },
            )
            .await
            .expect("pause");

        sync_handler_schedules(
            &engine_with(&store, "deploy", "0 */6 * * *"),
            store.as_ref(),
        )
        .await
        .expect("second sync");

        let s = reload(&store, id).await;
        assert!(!s.is_active(), "a user pause survives a cron change");
        assert!(s.next_trigger_at.is_some());
    }

    #[tokio::test]
    async fn repair_gives_a_next_trigger_to_an_active_schedule_without_one() {
        let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
        let broken = seed(&store, "hourly", "0 * * * *").await;
        let paused = seed(&store, "paused", "0 * * * *").await;
        store
            .update_schedule(
                paused.id,
                ScheduleUpdate {
                    disabled_at: Some(Some(Utc::now())),
                    ..Default::default()
                },
            )
            .await
            .expect("pause");

        let repaired = repair_unscheduled_schedules(store.as_ref())
            .await
            .expect("repair");

        assert_eq!(repaired, 1);
        let s = reload(&store, broken.id).await;
        assert!(s.is_active());
        assert!(s.next_trigger_at.expect("next trigger") > Utc::now());
        assert!(s.last_error.is_none());
        // A paused schedule needs no next trigger: left alone.
        let p = reload(&store, paused.id).await;
        assert!(!p.is_active());
        assert!(p.next_trigger_at.is_none());
    }

    #[tokio::test]
    async fn repair_disables_a_schedule_whose_next_trigger_cannot_be_computed() {
        let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
        let broken = seed(&store, "leap", "0 0 30 2 *").await;

        let repaired = repair_unscheduled_schedules(store.as_ref())
            .await
            .expect("repair");

        assert_eq!(repaired, 1);
        let s = reload(&store, broken.id).await;
        assert!(!s.is_active());
        let error = s.last_error.expect("disable reason stored");
        assert!(error.contains("cannot compute next trigger"), "{error}");
    }

    #[tokio::test]
    async fn repair_reads_every_page() {
        let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
        let count = REPAIR_PAGE_SIZE as usize * 2 + 3;
        for i in 0..count {
            seed(&store, &format!("wf-{i}"), "0 * * * *").await;
        }

        let repaired = repair_unscheduled_schedules(store.as_ref())
            .await
            .expect("repair");

        assert_eq!(repaired, count);
        let left = store
            .list_schedules(1, 1000)
            .await
            .expect("list")
            .items
            .into_iter()
            .filter(|s| s.is_active() && s.next_trigger_at.is_none())
            .count();
        assert_eq!(left, 0);
    }

    #[tokio::test]
    async fn repair_with_nothing_to_repair_changes_nothing() {
        let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
        let repaired = repair_unscheduled_schedules(store.as_ref())
            .await
            .expect("repair");
        assert_eq!(repaired, 0);
    }

    struct PriorityScheduled {
        priority: i16,
        cron: CronSchedule,
    }
    impl WorkflowHandler for PriorityScheduled {
        fn name(&self) -> &str {
            "urgent-report"
        }
        fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
            Box::pin(async { Ok(()) })
        }
        fn schedule(&self) -> Option<&CronSchedule> {
            Some(&self.cron)
        }
        fn priority(&self) -> i16 {
            self.priority
        }
    }

    fn engine_with_priority(store: &Arc<dyn Store>, priority: i16) -> Engine {
        let provider = Arc::new(ClaudeCodeProvider::new());
        let mut engine = Engine::new(store.clone(), provider);
        engine
            .register(PriorityScheduled {
                priority,
                cron: CronSchedule::new("0 0 * * *").unwrap(),
            })
            .expect("register");
        engine
    }

    async fn only_schedule(store: &Arc<dyn Store>) -> Schedule {
        let page = store.list_schedules(1, 10).await.expect("list");
        assert_eq!(page.items.len(), 1);
        page.items.into_iter().next().expect("one schedule")
    }

    #[tokio::test]
    async fn sync_creates_a_handler_schedule_with_the_handler_priority() {
        let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
        let engine = engine_with_priority(&store, 40);

        sync_handler_schedules(&engine, store.as_ref())
            .await
            .expect("sync");

        let schedule = only_schedule(&store).await;
        assert_eq!(schedule.source, ScheduleSource::Handler);
        assert_eq!(schedule.priority, 40);
        assert_eq!(schedule.new_run(None).priority, 40);
    }

    #[tokio::test]
    async fn sync_updates_the_priority_when_the_handler_priority_changed() {
        let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
        sync_handler_schedules(&engine_with_priority(&store, 0), store.as_ref())
            .await
            .expect("first sync");
        let before = only_schedule(&store).await;
        assert_eq!(before.priority, 0);

        sync_handler_schedules(&engine_with_priority(&store, -30), store.as_ref())
            .await
            .expect("second sync");

        let after = only_schedule(&store).await;
        assert_eq!(after.id, before.id);
        assert_eq!(after.priority, -30);
        assert_eq!(after.cron_expression, before.cron_expression);
        assert_eq!(after.next_trigger_at, before.next_trigger_at);
    }

    #[tokio::test]
    async fn sync_clamps_an_out_of_range_handler_priority() {
        let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
        sync_handler_schedules(&engine_with_priority(&store, 500), store.as_ref())
            .await
            .expect("sync high");
        assert_eq!(only_schedule(&store).await.priority, MAX_PRIORITY);

        sync_handler_schedules(&engine_with_priority(&store, -500), store.as_ref())
            .await
            .expect("sync low");
        assert_eq!(only_schedule(&store).await.priority, MIN_PRIORITY);
    }
}
