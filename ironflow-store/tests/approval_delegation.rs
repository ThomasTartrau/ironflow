//! Integration tests for approval delegations on the in-memory store.
//!
//! These drive `create_delegation` / `list_active_delegations` /
//! `find_delegation_by_id` / `delete_delegation` through the public
//! `ApprovalDelegationStore` API and never reach into the store's internals.

use chrono::{TimeDelta, Utc};
use ironflow_store::prelude::*;
use uuid::Uuid;

/// A delegation active right now, covering every workflow.
fn active(from: Uuid, to: Uuid) -> NewApprovalDelegation {
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
async fn one_delegator_can_cover_two_delegates_on_different_workflows() {
    let store = InMemoryStore::new();
    let (alice, bob, carol) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());

    store
        .create_delegation(NewApprovalDelegation {
            workflow_filter: Some("deploy-*".to_string()),
            ..active(alice, bob)
        })
        .await
        .expect("delegate deploys to bob");
    store
        .create_delegation(NewApprovalDelegation {
            workflow_filter: Some("cleanup-*".to_string()),
            ..active(alice, carol)
        })
        .await
        .expect("delegate cleanups to carol");

    let granted = store
        .list_active_delegations(DelegationFilter {
            from_user_id: Some(alice),
            ..DelegationFilter::default()
        })
        .await
        .expect("list");
    assert_eq!(granted.len(), 2);

    let now = Utc::now();
    let deploy_holder = granted
        .iter()
        .find(|d| d.covers("deploy-prod", now))
        .expect("someone covers deploy-prod");
    assert_eq!(deploy_holder.to_user_id, bob);

    let cleanup_holder = granted
        .iter()
        .find(|d| d.covers("cleanup-nightly", now))
        .expect("someone covers cleanup-nightly");
    assert_eq!(cleanup_holder.to_user_id, carol);

    assert!(
        !granted.iter().any(|d| d.covers("build", now)),
        "neither filter matches an unrelated workflow"
    );
}

#[tokio::test]
async fn one_delegate_can_hold_delegations_from_two_delegators() {
    let store = InMemoryStore::new();
    let (alice, carol, bob) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());

    store
        .create_delegation(active(alice, bob))
        .await
        .expect("alice -> bob");
    store
        .create_delegation(active(carol, bob))
        .await
        .expect("carol -> bob");

    let received = store
        .list_active_delegations(DelegationFilter {
            to_user_id: Some(bob),
            ..DelegationFilter::default()
        })
        .await
        .expect("list");

    let delegators: Vec<_> = received.iter().map(|d| d.from_user_id).collect();
    assert_eq!(received.len(), 2);
    assert!(delegators.contains(&alice));
    assert!(delegators.contains(&carol));
}

#[tokio::test]
async fn an_expired_delegation_is_ignored_next_to_a_fresh_one() {
    let store = InMemoryStore::new();
    let (alice, bob) = (Uuid::now_v7(), Uuid::now_v7());
    let now = Utc::now();

    store
        .create_delegation(NewApprovalDelegation {
            valid_from: now - TimeDelta::days(10),
            valid_until: now - TimeDelta::days(3),
            ..active(alice, bob)
        })
        .await
        .expect("expired delegation");
    let fresh = store
        .create_delegation(active(alice, bob))
        .await
        .expect("fresh delegation");

    let received = store
        .list_active_delegations(DelegationFilter {
            to_user_id: Some(bob),
            ..DelegationFilter::default()
        })
        .await
        .expect("list");

    assert_eq!(received.len(), 1);
    assert_eq!(received[0].id, fresh.id);
}

#[tokio::test]
async fn an_expired_delegation_stays_readable_by_id_so_it_can_be_revoked() {
    let store = InMemoryStore::new();
    let now = Utc::now();

    let expired = store
        .create_delegation(NewApprovalDelegation {
            valid_from: now - TimeDelta::days(10),
            valid_until: now - TimeDelta::days(3),
            ..active(Uuid::now_v7(), Uuid::now_v7())
        })
        .await
        .expect("expired delegation");

    let found = store
        .find_delegation_by_id(expired.id)
        .await
        .expect("find")
        .expect("expired rows are not deleted");
    assert_eq!(found.id, expired.id);

    store
        .delete_delegation(expired.id)
        .await
        .expect("an expired delegation can still be revoked");
}

#[tokio::test]
async fn revoking_one_delegation_leaves_the_others_intact() {
    let store = InMemoryStore::new();
    let (alice, bob, carol) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());

    let to_bob = store
        .create_delegation(active(alice, bob))
        .await
        .expect("alice -> bob");
    let to_carol = store
        .create_delegation(active(alice, carol))
        .await
        .expect("alice -> carol");

    store.delete_delegation(to_bob.id).await.expect("revoke");

    let remaining = store
        .list_active_delegations(DelegationFilter {
            from_user_id: Some(alice),
            ..DelegationFilter::default()
        })
        .await
        .expect("list");
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, to_carol.id);

    let err = store.delete_delegation(to_bob.id).await.unwrap_err();
    assert!(matches!(err, StoreError::DelegationNotFound(_)));
}
