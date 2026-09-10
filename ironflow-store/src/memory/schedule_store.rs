//! In-memory [`ScheduleStore`] implementation.

use chrono::Utc;
use uuid::Uuid;

use crate::entities::{NewSchedule, Page, Schedule, ScheduleUpdate};
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
                disabled_at: None,
                last_triggered_at: None,
                next_trigger_at: req.next_trigger_at,
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
            let due = state
                .schedules
                .values()
                .filter(|s| s.is_active() && s.next_trigger_at.is_some_and(|at| at <= now))
                .cloned()
                .collect();
            Ok(due)
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn new_schedule(workflow: &str, cron: &str) -> NewSchedule {
        NewSchedule {
            workflow_name: workflow.to_string(),
            cron_expression: cron.to_string(),
            inputs: json!({}),
            created_by_user_id: Uuid::now_v7(),
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

        let found = store
            .find_schedule_by_id(created.id)
            .await
            .expect("find")
            .expect("some");
        assert_eq!(found.id, created.id);
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

    #[tokio::test]
    async fn list_due_schedules_filters_correctly() {
        use chrono::TimeDelta;

        let store = InMemoryStore::new();

        let past = store
            .create_schedule(NewSchedule {
                workflow_name: "past".to_string(),
                cron_expression: "0 0 * * * *".to_string(),
                inputs: json!({}),
                created_by_user_id: Uuid::now_v7(),
                next_trigger_at: Some(Utc::now() - TimeDelta::seconds(60)),
            })
            .await
            .expect("create past");

        let _future = store
            .create_schedule(NewSchedule {
                workflow_name: "future".to_string(),
                cron_expression: "0 0 * * * *".to_string(),
                inputs: json!({}),
                created_by_user_id: Uuid::now_v7(),
                next_trigger_at: Some(Utc::now() + TimeDelta::seconds(3600)),
            })
            .await
            .expect("create future");

        let disabled = store
            .create_schedule(NewSchedule {
                workflow_name: "disabled".to_string(),
                cron_expression: "0 0 * * * *".to_string(),
                inputs: json!({}),
                created_by_user_id: Uuid::now_v7(),
                next_trigger_at: Some(Utc::now() - TimeDelta::seconds(60)),
            })
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

        let due = store.list_due_schedules().await.expect("due");
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].id, past.id);
    }
}
