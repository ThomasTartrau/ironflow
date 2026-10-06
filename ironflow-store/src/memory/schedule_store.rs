//! In-memory [`ScheduleStore`] implementation.

use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::run_store::insert_run;
use crate::entities::{NewSchedule, Page, Schedule, ScheduleFiring, ScheduleNext, ScheduleUpdate};
use crate::error::StoreError;
use crate::memory::InMemoryStore;
use crate::schedule_store::ScheduleStore;
use crate::store::StoreFuture;

impl ScheduleStore for InMemoryStore {
    fn create_schedule(&self, req: NewSchedule) -> StoreFuture<'_, Schedule> {
        Box::pin(async move {
            let now = Utc::now();
            let schedule = Schedule {
                id: Uuid::now_v7(),
                workflow_name: req.workflow_name,
                cron_expression: req.cron_expression,
                inputs: req.inputs,
                source: req.source,
                disabled_at: None,
                last_triggered_at: None,
                next_trigger_at: req.next_trigger_at,
                last_error: None,
                priority: req.priority,
                created_by_user_id: req.created_by_user_id,
                created_at: now,
                updated_at: now,
            };
            let mut state = self.state.write().await;
            state.schedules.insert(schedule.id, schedule.clone());
            Ok(schedule)
        })
    }

    fn find_schedule_by_id(&self, id: Uuid) -> StoreFuture<'_, Option<Schedule>> {
        Box::pin(async move {
            let state = self.state.read().await;
            Ok(state.schedules.get(&id).cloned())
        })
    }

    fn list_schedules(&self, page: u32, per_page: u32) -> StoreFuture<'_, Page<Schedule>> {
        Box::pin(async move {
            let state = self.state.read().await;
            let mut all: Vec<_> = state.schedules.values().cloned().collect();
            all.sort_by_key(|s| std::cmp::Reverse(s.created_at));
            let total = all.len() as u64;
            let start = ((page.saturating_sub(1)) as usize) * (per_page as usize);
            let items: Vec<_> = all
                .into_iter()
                .skip(start)
                .take(per_page as usize)
                .collect();
            Ok(Page {
                items,
                total,
                page,
                per_page,
            })
        })
    }

    fn update_schedule(&self, id: Uuid, update: ScheduleUpdate) -> StoreFuture<'_, Schedule> {
        Box::pin(async move {
            let mut state = self.state.write().await;
            let schedule = state
                .schedules
                .get_mut(&id)
                .ok_or(StoreError::ScheduleNotFound(id))?;

            if let Some(cron) = update.cron_expression {
                schedule.cron_expression = cron;
            }
            if let Some(inputs) = update.inputs {
                schedule.inputs = inputs;
            }
            if let Some(disabled) = update.disabled_at {
                schedule.disabled_at = disabled;
            }
            if let Some(next) = update.next_trigger_at {
                schedule.next_trigger_at = next;
            }
            if let Some(last) = update.last_triggered_at {
                schedule.last_triggered_at = last;
            }
            if let Some(error) = update.last_error {
                schedule.last_error = error;
            }
            if let Some(priority) = update.priority {
                schedule.priority = priority;
            }
            schedule.updated_at = Utc::now();
            Ok(schedule.clone())
        })
    }

    fn delete_schedule(&self, id: Uuid) -> StoreFuture<'_, ()> {
        Box::pin(async move {
            let mut state = self.state.write().await;
            state
                .schedules
                .remove(&id)
                .ok_or(StoreError::ScheduleNotFound(id))?;
            Ok(())
        })
    }

    fn list_due_schedules(&self) -> StoreFuture<'_, Vec<Schedule>> {
        Box::pin(async move {
            let now = Utc::now();
            let state = self.state.read().await;
            let mut due: Vec<Schedule> = state
                .schedules
                .values()
                .filter(|s| s.is_active() && s.next_trigger_at.is_some_and(|at| at <= now))
                .cloned()
                .collect();
            due.sort_by_key(|s| s.next_trigger_at);
            Ok(due)
        })
    }

    fn fire_due_schedule(
        &self,
        id: Uuid,
        occurrence: DateTime<Utc>,
        next: ScheduleNext,
    ) -> StoreFuture<'_, Option<ScheduleFiring>> {
        Box::pin(async move {
            // One write lock covers the check, the run and the schedule update.
            let mut state = self.state.write().await;
            let Some(schedule) = state
                .schedules
                .get(&id)
                .filter(|s| s.is_active() && s.next_trigger_at == Some(occurrence))
            else {
                return Ok(None);
            };

            let mut new_run = schedule.new_run(None);
            new_run.idempotency_key = Some(Schedule::occurrence_key(id, occurrence));
            let run = insert_run(&mut state, new_run)?;

            let now = Utc::now();
            let schedule = state
                .schedules
                .get_mut(&id)
                .ok_or(StoreError::ScheduleNotFound(id))?;
            schedule.last_triggered_at = Some(now);
            schedule.updated_at = now;
            match next {
                ScheduleNext::At(at) => schedule.next_trigger_at = Some(at),
                ScheduleNext::Disable { error } => {
                    schedule.next_trigger_at = None;
                    schedule.disabled_at = Some(now);
                    schedule.last_error = Some(error);
                }
            }

            Ok(Some(ScheduleFiring {
                schedule: schedule.clone(),
                run,
            }))
        })
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeDelta;
    use serde_json::json;

    use crate::entities::{RunFilter, ScheduleSource};
    use crate::store::RunStore;

    use super::*;

    fn new_schedule(workflow: &str, cron: &str) -> NewSchedule {
        NewSchedule {
            workflow_name: workflow.to_string(),
            cron_expression: cron.to_string(),
            inputs: json!({}),
            source: ScheduleSource::Api,
            priority: 0,
            created_by_user_id: Some(Uuid::now_v7()),
            next_trigger_at: Some(Utc::now()),
        }
    }

    #[tokio::test]
    async fn create_and_find() {
        let store = InMemoryStore::new();
        let created = store
            .create_schedule(new_schedule("deploy", "0 0 * * * *"))
            .await
            .expect("create");
        assert_eq!(created.workflow_name, "deploy");
        assert!(created.is_active());
        assert_eq!(created.source, ScheduleSource::Api);

        let found = store
            .find_schedule_by_id(created.id)
            .await
            .expect("find")
            .expect("some");
        assert_eq!(found.id, created.id);
    }

    #[tokio::test]
    async fn create_handler_source() {
        let store = InMemoryStore::new();
        let created = store
            .create_schedule(NewSchedule {
                source: ScheduleSource::Handler,
                ..new_schedule("nightly", "0 0 * * *")
            })
            .await
            .expect("create");
        assert_eq!(created.source, ScheduleSource::Handler);
    }

    #[tokio::test]
    async fn list_paginated() {
        let store = InMemoryStore::new();
        for i in 0..5 {
            store
                .create_schedule(new_schedule(&format!("wf-{i}"), "0 0 * * * *"))
                .await
                .expect("create");
        }
        let page = store.list_schedules(1, 3).await.expect("list");
        assert_eq!(page.items.len(), 3);
        assert_eq!(page.total, 5);

        let page2 = store.list_schedules(2, 3).await.expect("list");
        assert_eq!(page2.items.len(), 2);
    }

    #[tokio::test]
    async fn update_fields() {
        let store = InMemoryStore::new();
        let created = store
            .create_schedule(new_schedule("deploy", "0 0 * * * *"))
            .await
            .expect("create");

        let updated = store
            .update_schedule(
                created.id,
                ScheduleUpdate {
                    disabled_at: Some(Some(Utc::now())),
                    cron_expression: Some("0 30 * * * *".to_string()),
                    ..Default::default()
                },
            )
            .await
            .expect("update");

        assert!(!updated.is_active());
        assert_eq!(updated.cron_expression, "0 30 * * * *");
    }

    #[tokio::test]
    async fn update_schedule_priority() {
        let store = InMemoryStore::new();
        let created = store
            .create_schedule(NewSchedule {
                priority: 5,
                ..new_schedule("deploy", "0 0 * * * *")
            })
            .await
            .expect("create");
        assert_eq!(created.priority, 5);

        let updated = store
            .update_schedule(
                created.id,
                ScheduleUpdate {
                    priority: Some(-20),
                    ..Default::default()
                },
            )
            .await
            .expect("update");
        assert_eq!(updated.priority, -20);
    }

    #[tokio::test]
    async fn fire_due_schedule_run_inherits_schedule_priority() {
        let store = InMemoryStore::new();
        let schedule = store
            .create_schedule(NewSchedule {
                priority: 30,
                ..due_schedule("deploy")
            })
            .await
            .expect("create");
        let occurrence = schedule.next_trigger_at.expect("due");

        let firing = store
            .fire_due_schedule(
                schedule.id,
                occurrence,
                ScheduleNext::At(Utc::now() + TimeDelta::seconds(3600)),
            )
            .await
            .expect("fire")
            .expect("due occurrence fires");

        assert_eq!(firing.run.run().priority, 30);
    }

    #[tokio::test]
    async fn update_not_found() {
        let store = InMemoryStore::new();
        let err = store
            .update_schedule(Uuid::now_v7(), ScheduleUpdate::default())
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::ScheduleNotFound(_)));
    }

    #[tokio::test]
    async fn delete_existing() {
        let store = InMemoryStore::new();
        let created = store
            .create_schedule(new_schedule("deploy", "0 0 * * * *"))
            .await
            .expect("create");

        store.delete_schedule(created.id).await.expect("delete");

        let found = store.find_schedule_by_id(created.id).await.expect("find");
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn delete_not_found() {
        let store = InMemoryStore::new();
        let err = store.delete_schedule(Uuid::now_v7()).await.unwrap_err();
        assert!(matches!(err, StoreError::ScheduleNotFound(_)));
    }

    fn due_schedule(workflow: &str) -> NewSchedule {
        NewSchedule {
            next_trigger_at: Some(Utc::now() - TimeDelta::seconds(60)),
            ..new_schedule(workflow, "0 0 * * * *")
        }
    }

    async fn run_count(store: &InMemoryStore) -> usize {
        store
            .list_runs(RunFilter::default(), 1, 100)
            .await
            .expect("list runs")
            .items
            .len()
    }

    #[tokio::test]
    async fn list_due_schedules_filters_and_changes_nothing() {
        let store = InMemoryStore::new();

        let past = store
            .create_schedule(due_schedule("past"))
            .await
            .expect("create past");
        store
            .create_schedule(NewSchedule {
                next_trigger_at: Some(Utc::now() + TimeDelta::seconds(3600)),
                ..new_schedule("future", "0 0 * * * *")
            })
            .await
            .expect("create future");
        let disabled = store
            .create_schedule(due_schedule("disabled"))
            .await
            .expect("create disabled");
        store
            .update_schedule(
                disabled.id,
                ScheduleUpdate {
                    disabled_at: Some(Some(Utc::now())),
                    ..Default::default()
                },
            )
            .await
            .expect("disable");

        let due = store.list_due_schedules().await.expect("list due");
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].id, past.id);

        // Listing is read-only: the schedule is still due at its occurrence.
        let again = store.list_due_schedules().await.expect("list due again");
        assert_eq!(again.len(), 1);
        assert_eq!(again[0].next_trigger_at, past.next_trigger_at);
        assert!(again[0].last_triggered_at.is_none());
    }

    #[tokio::test]
    async fn fire_due_schedule_creates_run_and_advances_next_trigger() {
        let store = InMemoryStore::new();
        let schedule = store
            .create_schedule(NewSchedule {
                inputs: json!({"env": "prod"}),
                ..due_schedule("deploy")
            })
            .await
            .expect("create");
        let occurrence = schedule.next_trigger_at.expect("due");
        let next = Utc::now() + TimeDelta::seconds(3600);

        let firing = store
            .fire_due_schedule(schedule.id, occurrence, ScheduleNext::At(next))
            .await
            .expect("fire")
            .expect("due occurrence fires");

        assert!(firing.run.is_created());
        let run = firing.run.run();
        assert_eq!(run.workflow_name, "deploy");
        assert_eq!(run.payload, json!({"env": "prod"}));
        assert_eq!(
            run.idempotency_key.as_deref(),
            Some(Schedule::occurrence_key(schedule.id, occurrence).as_str())
        );
        assert_eq!(firing.schedule.next_trigger_at, Some(next));
        assert!(firing.schedule.last_triggered_at.is_some());
        assert!(firing.schedule.is_active());

        let stored = store
            .find_schedule_by_id(schedule.id)
            .await
            .expect("find")
            .expect("exists");
        assert_eq!(stored.next_trigger_at, Some(next));
        assert!(store.list_due_schedules().await.expect("due").is_empty());
    }

    #[tokio::test]
    async fn fire_same_occurrence_twice_creates_one_run() {
        let store = InMemoryStore::new();
        let schedule = store
            .create_schedule(due_schedule("deploy"))
            .await
            .expect("create");
        let occurrence = schedule.next_trigger_at.expect("due");
        let next = ScheduleNext::At(Utc::now() + TimeDelta::seconds(3600));

        let first = store
            .fire_due_schedule(schedule.id, occurrence, next.clone())
            .await
            .expect("first fire");
        let second = store
            .fire_due_schedule(schedule.id, occurrence, next)
            .await
            .expect("second fire");

        assert!(first.is_some());
        assert!(second.is_none(), "the occurrence was already fired");
        assert_eq!(run_count(&store).await, 1);
    }

    #[tokio::test]
    async fn fire_reuses_the_run_already_bound_to_the_occurrence_key() {
        let store = InMemoryStore::new();
        let schedule = store
            .create_schedule(due_schedule("deploy"))
            .await
            .expect("create");
        let occurrence = schedule.next_trigger_at.expect("due");
        let mut earlier = schedule.new_run(None);
        earlier.idempotency_key = Some(Schedule::occurrence_key(schedule.id, occurrence));
        let earlier = store.create_run(earlier).await.expect("earlier run");

        let firing = store
            .fire_due_schedule(
                schedule.id,
                occurrence,
                ScheduleNext::At(Utc::now() + TimeDelta::seconds(3600)),
            )
            .await
            .expect("fire")
            .expect("due occurrence fires");

        assert!(!firing.run.is_created());
        assert_eq!(firing.run.run().id, earlier.run().id);
        assert_eq!(run_count(&store).await, 1);
        assert!(firing.schedule.next_trigger_at.is_some());
    }

    #[tokio::test]
    async fn fire_paused_schedule_returns_none() {
        let store = InMemoryStore::new();
        let schedule = store
            .create_schedule(due_schedule("deploy"))
            .await
            .expect("create");
        let occurrence = schedule.next_trigger_at.expect("due");
        store
            .update_schedule(
                schedule.id,
                ScheduleUpdate {
                    disabled_at: Some(Some(Utc::now())),
                    ..Default::default()
                },
            )
            .await
            .expect("pause");

        let fired = store
            .fire_due_schedule(
                schedule.id,
                occurrence,
                ScheduleNext::At(Utc::now() + TimeDelta::seconds(3600)),
            )
            .await
            .expect("fire");

        assert!(fired.is_none());
        assert_eq!(run_count(&store).await, 0);
    }

    #[tokio::test]
    async fn fire_unknown_schedule_returns_none() {
        let store = InMemoryStore::new();
        let fired = store
            .fire_due_schedule(Uuid::now_v7(), Utc::now(), ScheduleNext::At(Utc::now()))
            .await
            .expect("fire");
        assert!(fired.is_none());
    }

    #[tokio::test]
    async fn fire_with_disable_creates_run_and_disables_with_error() {
        let store = InMemoryStore::new();
        let schedule = store
            .create_schedule(due_schedule("deploy"))
            .await
            .expect("create");
        let occurrence = schedule.next_trigger_at.expect("due");

        let firing = store
            .fire_due_schedule(
                schedule.id,
                occurrence,
                ScheduleNext::Disable {
                    error: "no next occurrence".to_string(),
                },
            )
            .await
            .expect("fire")
            .expect("due occurrence fires");

        assert!(firing.run.is_created());
        let stored = store
            .find_schedule_by_id(schedule.id)
            .await
            .expect("find")
            .expect("exists");
        assert!(!stored.is_active());
        assert!(stored.next_trigger_at.is_none());
        assert_eq!(stored.last_error.as_deref(), Some("no next occurrence"));
    }

    #[tokio::test]
    async fn update_sets_and_clears_last_error() {
        let store = InMemoryStore::new();
        let created = store
            .create_schedule(new_schedule("deploy", "0 0 * * * *"))
            .await
            .expect("create");
        assert!(created.last_error.is_none());

        let set = store
            .update_schedule(
                created.id,
                ScheduleUpdate {
                    last_error: Some(Some("boom".to_string())),
                    ..Default::default()
                },
            )
            .await
            .expect("set");
        assert_eq!(set.last_error.as_deref(), Some("boom"));

        let cleared = store
            .update_schedule(
                created.id,
                ScheduleUpdate {
                    last_error: Some(None),
                    ..Default::default()
                },
            )
            .await
            .expect("clear");
        assert!(cleared.last_error.is_none());
    }
}
