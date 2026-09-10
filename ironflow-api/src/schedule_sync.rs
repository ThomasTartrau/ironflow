//! Reconciliation of handler-declared schedules with the database.
//!
//! Call [`sync_handler_schedules`] once at server startup, before spawning
//! the [`ScheduleTicker`](super::schedule_ticker::ScheduleTicker).

use std::collections::{HashMap, HashSet};

use ironflow_engine::engine::Engine;
use ironflow_store::entities::{NewSchedule, ScheduleSource, ScheduleUpdate};
use ironflow_store::store::Store;
use tracing::info;
use uuid::Uuid;

use crate::schedule_ticker::next_trigger;

/// Reconcile DB schedules with handler-declared schedules.
///
/// Performs a three-way sync at startup:
/// 1. **Create** DB rows for handlers that declare a schedule but have no
///    corresponding `source = handler` row.
/// 2. **Update** the cron expression when the handler's cron changed.
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
pub async fn sync_handler_schedules(
    engine: &Engine,
    store: &dyn Store,
) -> Result<(), ironflow_store::error::StoreError> {
    let handlers = engine.scheduled_handlers();
    let handler_map: HashMap<&str, &str> = handlers
        .iter()
        .map(|(name, cron)| (*name, cron.as_str()))
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
    let existing_map: HashMap<&str, &ironflow_store::entities::Schedule> = handler_schedules
        .iter()
        .filter(|s| handler_names.contains(s.workflow_name.as_str()))
        .map(|s| (s.workflow_name.as_str(), *s))
        .collect();

    for (name, cron_str) in &handler_map {
        match existing_map.get(name) {
            Some(existing) if existing.cron_expression != *cron_str => {
                // 2. Cron changed in code: update DB row.
                let next = next_trigger(cron_str).unwrap_or(None);
                store
                    .update_schedule(
                        existing.id,
                        ScheduleUpdate {
                            cron_expression: Some(cron_str.to_string()),
                            next_trigger_at: Some(next),
                            ..Default::default()
                        },
                    )
                    .await?;
                info!(
                    workflow = %name,
                    old_cron = %existing.cron_expression,
                    new_cron = %cron_str,
                    "updated handler schedule cron"
                );
            }
            Some(_) => {}
            None => {
                // 3. Missing: create a new handler schedule.
                let next = next_trigger(cron_str).unwrap_or(None);
                store
                    .create_schedule(NewSchedule {
                        workflow_name: name.to_string(),
                        cron_expression: cron_str.to_string(),
                        inputs: serde_json::json!({}),
                        source: ScheduleSource::Handler,
                        created_by_user_id: Uuid::nil(),
                        next_trigger_at: next,
                    })
                    .await?;
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

#[cfg(test)]
mod tests {
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::context::WorkflowContext;
    use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
    use ironflow_engine::prelude::CronSchedule;
    use ironflow_store::entities::{NewSchedule, ScheduleSource};
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::store::Store;
    use serde_json::json;
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
                created_by_user_id: Uuid::now_v7(),
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
                created_by_user_id: Uuid::nil(),
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
                created_by_user_id: Uuid::now_v7(),
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
                created_by_user_id: Uuid::nil(),
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
}
