//! [`AuditLogStore`] trait implementation for [`InMemoryStore`].

use chrono::Utc;
use uuid::Uuid;

use crate::audit_log_store::AuditLogStore;
use crate::entities::{AuditLogEntry, AuditLogFilter, NewAuditLogEntry, Page};
use crate::store::StoreFuture;

use super::InMemoryStore;

impl AuditLogStore for InMemoryStore {
    fn append_audit_log(&self, entry: NewAuditLogEntry) -> StoreFuture<'_, AuditLogEntry> {
        Box::pin(async move {
            let now = Utc::now();
            let audit_entry = AuditLogEntry {
                id: Uuid::now_v7(),
                event_type: entry.event_type,
                payload: entry.payload,
                run_id: entry.run_id,
                step_id: entry.step_id,
                user_id: entry.user_id,
                created_at: now,
            };

            let mut state = self.state.write().await;
            state.audit_logs.push(audit_entry.clone());
            Ok(audit_entry)
        })
    }

    fn list_audit_logs(
        &self,
        filter: AuditLogFilter,
        page: u32,
        per_page: u32,
    ) -> StoreFuture<'_, Page<AuditLogEntry>> {
        Box::pin(async move {
            let state = self.state.read().await;

            let filtered: Vec<&AuditLogEntry> = state
                .audit_logs
                .iter()
                .filter(|e| {
                    if let Some(event_type) = filter.event_type
                        && e.event_type != event_type
                    {
                        return false;
                    }
                    if let Some(run_id) = filter.run_id
                        && e.run_id != Some(run_id)
                    {
                        return false;
                    }
                    if let Some(from) = filter.from
                        && e.created_at < from
                    {
                        return false;
                    }
                    if let Some(to) = filter.to
                        && e.created_at > to
                    {
                        return false;
                    }
                    true
                })
                .collect();

            let total = filtered.len() as u64;

            let offset = (page.saturating_sub(1) as usize) * (per_page as usize);
            let items: Vec<AuditLogEntry> = filtered
                .into_iter()
                .rev()
                .skip(offset)
                .take(per_page as usize)
                .cloned()
                .collect();

            Ok(Page {
                items,
                total,
                page,
                per_page,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use uuid::Uuid;

    use crate::audit_log_store::AuditLogStore;
    use crate::entities::{AuditLogFilter, EventKind, NewAuditLogEntry};
    use crate::memory::InMemoryStore;

    fn new_entry(event_type: EventKind, run_id: Option<Uuid>) -> NewAuditLogEntry {
        NewAuditLogEntry {
            event_type,
            payload: json!({"test": true}),
            run_id,
            step_id: None,
            user_id: None,
        }
    }

    #[tokio::test]
    async fn append_returns_entry_with_id() {
        let store = InMemoryStore::new();
        let entry = store
            .append_audit_log(new_entry(EventKind::RunCreated, None))
            .await
            .unwrap();

        assert_eq!(entry.event_type, EventKind::RunCreated);
        assert!(!entry.id.is_nil());
    }

    #[tokio::test]
    async fn list_empty_store_returns_empty_page() {
        let store = InMemoryStore::new();
        let page = store
            .list_audit_logs(AuditLogFilter::default(), 1, 20)
            .await
            .unwrap();

        assert!(page.items.is_empty());
        assert_eq!(page.total, 0);
    }

    #[tokio::test]
    async fn list_returns_all_entries() {
        let store = InMemoryStore::new();
        store
            .append_audit_log(new_entry(EventKind::RunCreated, None))
            .await
            .unwrap();
        store
            .append_audit_log(new_entry(EventKind::RunFailed, None))
            .await
            .unwrap();

        let page = store
            .list_audit_logs(AuditLogFilter::default(), 1, 20)
            .await
            .unwrap();

        assert_eq!(page.items.len(), 2);
        assert_eq!(page.total, 2);
    }

    #[tokio::test]
    async fn list_newest_first() {
        let store = InMemoryStore::new();
        store
            .append_audit_log(new_entry(EventKind::RunCreated, None))
            .await
            .unwrap();
        store
            .append_audit_log(new_entry(EventKind::RunFailed, None))
            .await
            .unwrap();

        let page = store
            .list_audit_logs(AuditLogFilter::default(), 1, 20)
            .await
            .unwrap();

        assert_eq!(page.items[0].event_type, EventKind::RunFailed);
        assert_eq!(page.items[1].event_type, EventKind::RunCreated);
    }

    #[tokio::test]
    async fn list_filters_by_event_type() {
        let store = InMemoryStore::new();
        store
            .append_audit_log(new_entry(EventKind::RunCreated, None))
            .await
            .unwrap();
        store
            .append_audit_log(new_entry(EventKind::RunFailed, None))
            .await
            .unwrap();
        store
            .append_audit_log(new_entry(EventKind::RunCreated, None))
            .await
            .unwrap();

        let filter = AuditLogFilter {
            event_type: Some(EventKind::RunCreated),
            ..AuditLogFilter::default()
        };
        let page = store.list_audit_logs(filter, 1, 20).await.unwrap();

        assert_eq!(page.items.len(), 2);
        assert_eq!(page.total, 2);
        assert!(
            page.items
                .iter()
                .all(|e| e.event_type == EventKind::RunCreated)
        );
    }

    #[tokio::test]
    async fn list_filters_by_run_id() {
        let store = InMemoryStore::new();
        let target_run = Uuid::now_v7();

        store
            .append_audit_log(new_entry(EventKind::RunCreated, Some(target_run)))
            .await
            .unwrap();
        store
            .append_audit_log(new_entry(EventKind::RunCreated, Some(Uuid::now_v7())))
            .await
            .unwrap();

        let filter = AuditLogFilter {
            run_id: Some(target_run),
            ..AuditLogFilter::default()
        };
        let page = store.list_audit_logs(filter, 1, 20).await.unwrap();

        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].run_id, Some(target_run));
    }

    #[tokio::test]
    async fn list_paginates_correctly() {
        let store = InMemoryStore::new();
        let kinds = [
            EventKind::RunCreated,
            EventKind::RunFailed,
            EventKind::StepCompleted,
            EventKind::StepFailed,
            EventKind::UserSignedIn,
        ];
        for kind in kinds {
            store.append_audit_log(new_entry(kind, None)).await.unwrap();
        }

        let page1 = store
            .list_audit_logs(AuditLogFilter::default(), 1, 2)
            .await
            .unwrap();
        assert_eq!(page1.items.len(), 2);
        assert_eq!(page1.total, 5);
        assert_eq!(page1.page, 1);

        let page2 = store
            .list_audit_logs(AuditLogFilter::default(), 2, 2)
            .await
            .unwrap();
        assert_eq!(page2.items.len(), 2);

        let page3 = store
            .list_audit_logs(AuditLogFilter::default(), 3, 2)
            .await
            .unwrap();
        assert_eq!(page3.items.len(), 1);
    }

    #[tokio::test]
    async fn list_filters_by_date_range() {
        let store = InMemoryStore::new();
        let entry1 = store
            .append_audit_log(new_entry(EventKind::RunCreated, None))
            .await
            .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let midpoint = chrono::Utc::now();

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let entry2 = store
            .append_audit_log(new_entry(EventKind::RunFailed, None))
            .await
            .unwrap();

        let filter_from = AuditLogFilter {
            from: Some(midpoint),
            ..AuditLogFilter::default()
        };
        let page = store.list_audit_logs(filter_from, 1, 20).await.unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].id, entry2.id);

        let filter_to = AuditLogFilter {
            to: Some(midpoint),
            ..AuditLogFilter::default()
        };
        let page = store.list_audit_logs(filter_to, 1, 20).await.unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].id, entry1.id);
    }

    #[tokio::test]
    async fn append_preserves_all_fields() {
        let store = InMemoryStore::new();
        let run_id = Uuid::now_v7();
        let step_id = Uuid::now_v7();
        let user_id = Uuid::now_v7();

        let entry = store
            .append_audit_log(NewAuditLogEntry {
                event_type: EventKind::StepCompleted,
                payload: json!({"cost": 0.42}),
                run_id: Some(run_id),
                step_id: Some(step_id),
                user_id: Some(user_id),
            })
            .await
            .unwrap();

        assert_eq!(entry.event_type, EventKind::StepCompleted);
        assert_eq!(entry.run_id, Some(run_id));
        assert_eq!(entry.step_id, Some(step_id));
        assert_eq!(entry.user_id, Some(user_id));
        assert_eq!(entry.payload, json!({"cost": 0.42}));
    }
}
