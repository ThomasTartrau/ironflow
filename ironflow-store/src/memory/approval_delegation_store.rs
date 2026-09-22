//! In-memory [`ApprovalDelegationStore`] implementation.

use std::cmp::Reverse;

use chrono::Utc;
use uuid::Uuid;

use crate::approval_delegation_store::ApprovalDelegationStore;
use crate::entities::{ApprovalDelegation, DelegationFilter, NewApprovalDelegation};
use crate::error::StoreError;
use crate::memory::InMemoryStore;
use crate::store::StoreFuture;

impl ApprovalDelegationStore for InMemoryStore {
    fn create_delegation(&self, req: NewApprovalDelegation) -> StoreFuture<'_, ApprovalDelegation> {
        Box::pin(async move {
            let delegation = ApprovalDelegation {
                id: Uuid::now_v7(),
                from_user_id: req.from_user_id,
                to_user_id: req.to_user_id,
                valid_from: req.valid_from,
                valid_until: req.valid_until,
                workflow_filter: req.workflow_filter,
                created_at: Utc::now(),
            };
            let mut state = self.state.write().await;
            state
                .approval_delegations
                .insert(delegation.id, delegation.clone());
            Ok(delegation)
        })
    }

    fn find_delegation_by_id(&self, id: Uuid) -> StoreFuture<'_, Option<ApprovalDelegation>> {
        Box::pin(async move {
            let state = self.state.read().await;
            Ok(state.approval_delegations.get(&id).cloned())
        })
    }

    fn list_active_delegations(
        &self,
        filter: DelegationFilter,
    ) -> StoreFuture<'_, Vec<ApprovalDelegation>> {
        Box::pin(async move {
            let now = Utc::now();
            let state = self.state.read().await;
            let mut items: Vec<_> = state
                .approval_delegations
                .values()
                .filter(|d| d.is_active_at(now))
                .filter(|d| filter.from_user_id.is_none_or(|id| d.from_user_id == id))
                .filter(|d| filter.to_user_id.is_none_or(|id| d.to_user_id == id))
                .cloned()
                .collect();
            items.sort_by_key(|d| Reverse(d.created_at));
            Ok(items)
        })
    }

    fn delete_delegation(&self, id: Uuid) -> StoreFuture<'_, ()> {
        Box::pin(async move {
            let mut state = self.state.write().await;
            state
                .approval_delegations
                .remove(&id)
                .ok_or(StoreError::DelegationNotFound(id))?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use chrono::TimeDelta;
    use tokio::time::sleep;

    use super::*;

    fn new_delegation(from: Uuid, to: Uuid) -> NewApprovalDelegation {
        let now = Utc::now();
        NewApprovalDelegation {
            from_user_id: from,
            to_user_id: to,
            valid_from: now - TimeDelta::hours(1),
            valid_until: now + TimeDelta::hours(1),
            workflow_filter: None,
        }
    }

    #[tokio::test]
    async fn create_and_find() {
        let store = InMemoryStore::new();
        let (alice, bob) = (Uuid::now_v7(), Uuid::now_v7());

        let created = store
            .create_delegation(NewApprovalDelegation {
                workflow_filter: Some("deploy-*".to_string()),
                ..new_delegation(alice, bob)
            })
            .await
            .expect("create");

        assert_eq!(created.from_user_id, alice);
        assert_eq!(created.to_user_id, bob);
        assert_eq!(created.workflow_filter.as_deref(), Some("deploy-*"));

        let found = store
            .find_delegation_by_id(created.id)
            .await
            .expect("find")
            .expect("some");
        assert_eq!(found.id, created.id);
    }

    #[tokio::test]
    async fn list_skips_expired_and_future_rows() {
        let store = InMemoryStore::new();
        let (alice, bob) = (Uuid::now_v7(), Uuid::now_v7());
        let now = Utc::now();

        let active = store
            .create_delegation(new_delegation(alice, bob))
            .await
            .expect("create active");
        store
            .create_delegation(NewApprovalDelegation {
                valid_from: now - TimeDelta::days(2),
                valid_until: now - TimeDelta::days(1),
                ..new_delegation(alice, bob)
            })
            .await
            .expect("create expired");
        store
            .create_delegation(NewApprovalDelegation {
                valid_from: now + TimeDelta::days(1),
                valid_until: now + TimeDelta::days(2),
                ..new_delegation(alice, bob)
            })
            .await
            .expect("create future");

        let listed = store
            .list_active_delegations(DelegationFilter::default())
            .await
            .expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, active.id);
    }

    #[tokio::test]
    async fn list_filters_by_delegate_and_by_delegator() {
        let store = InMemoryStore::new();
        let (alice, bob, carol) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());

        let to_bob = store
            .create_delegation(new_delegation(alice, bob))
            .await
            .expect("create");
        let to_carol = store
            .create_delegation(new_delegation(alice, carol))
            .await
            .expect("create");
        let from_carol = store
            .create_delegation(new_delegation(carol, bob))
            .await
            .expect("create");

        let received_by_bob = store
            .list_active_delegations(DelegationFilter {
                to_user_id: Some(bob),
                ..DelegationFilter::default()
            })
            .await
            .expect("list");
        let ids: Vec<_> = received_by_bob.iter().map(|d| d.id).collect();
        assert_eq!(received_by_bob.len(), 2);
        assert!(ids.contains(&to_bob.id));
        assert!(ids.contains(&from_carol.id));

        let granted_by_alice = store
            .list_active_delegations(DelegationFilter {
                from_user_id: Some(alice),
                ..DelegationFilter::default()
            })
            .await
            .expect("list");
        let ids: Vec<_> = granted_by_alice.iter().map(|d| d.id).collect();
        assert_eq!(granted_by_alice.len(), 2);
        assert!(ids.contains(&to_bob.id));
        assert!(ids.contains(&to_carol.id));
    }

    #[tokio::test]
    async fn list_returns_the_newest_delegation_first() {
        let store = InMemoryStore::new();
        let (alice, bob, carol) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());

        let first = store
            .create_delegation(new_delegation(alice, bob))
            .await
            .expect("create");
        // `created_at` is stamped by the store, so the two rows need a real gap
        // for the ordering assertion to mean anything.
        sleep(Duration::from_millis(5)).await;
        let second = store
            .create_delegation(new_delegation(alice, carol))
            .await
            .expect("create");

        let listed = store
            .list_active_delegations(DelegationFilter::default())
            .await
            .expect("list");
        assert_eq!(listed[0].id, second.id);
        assert_eq!(listed[1].id, first.id);
    }

    #[tokio::test]
    async fn delete_removes_the_row() {
        let store = InMemoryStore::new();
        let created = store
            .create_delegation(new_delegation(Uuid::now_v7(), Uuid::now_v7()))
            .await
            .expect("create");

        store.delete_delegation(created.id).await.expect("delete");

        let found = store
            .find_delegation_by_id(created.id)
            .await
            .expect("find delegation");
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn delete_unknown_id_is_not_found() {
        let store = InMemoryStore::new();
        let err = store.delete_delegation(Uuid::now_v7()).await.unwrap_err();
        assert!(matches!(err, StoreError::DelegationNotFound(_)));
    }
}
