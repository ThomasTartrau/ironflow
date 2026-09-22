#![cfg(feature = "store-postgres")]

//! Integration tests for approval delegations on PostgreSQL.
//!
//! These tests need a real database (`DATABASE_URL`) because what they check --
//! the foreign keys onto `iam.users`, the CHECK constraints, and the active
//! window evaluated in SQL -- lives in the schema, not in Rust.
//!
//! Run them with:
//!
//! ```sh
//! DATABASE_URL=postgres://... cargo test -p ironflow-store --features store-postgres --test postgres_approval_delegation -- --ignored
//! ```

use std::env::var;

use chrono::{TimeDelta, Utc};
use ironflow_store::approval_delegation_store::ApprovalDelegationStore;
use ironflow_store::entities::{DelegationFilter, NewApprovalDelegation, NewUser};
use ironflow_store::error::StoreError;
use ironflow_store::postgres::PostgresStore;
use ironflow_store::user_store::UserStore;
use uuid::Uuid;

async fn get_store() -> PostgresStore {
    let url = var("DATABASE_URL").expect("DATABASE_URL must be set");
    PostgresStore::new(&url)
        .await
        .expect("failed to connect to PostgreSQL")
}

/// Create a real user: the delegation table has a foreign key onto `iam.users`.
///
/// The suffix keeps reruns from colliding on the unique email and username.
async fn create_user(store: &PostgresStore, prefix: &str) -> Uuid {
    let suffix = Uuid::now_v7();
    store
        .create_user(NewUser {
            email: format!("{prefix}-{suffix}@example.com"),
            username: format!("{prefix}-{suffix}"),
            password_hash: "not-a-real-hash".to_string(),
            is_admin: Some(false),
        })
        .await
        .expect("create user")
        .id
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn create_and_find_roundtrip() {
    let store = get_store().await;
    let alice = create_user(&store, "alice").await;
    let bob = create_user(&store, "bob").await;
    let now = Utc::now();

    let created = store
        .create_delegation(NewApprovalDelegation {
            from_user_id: alice,
            to_user_id: bob,
            valid_from: now,
            valid_until: now + TimeDelta::days(7),
            workflow_filter: Some("deploy-*".to_string()),
        })
        .await
        .expect("create");

    let found = store
        .find_delegation_by_id(created.id)
        .await
        .expect("find")
        .expect("some");
    assert_eq!(found.from_user_id, alice);
    assert_eq!(found.to_user_id, bob);
    assert_eq!(found.workflow_filter.as_deref(), Some("deploy-*"));
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn a_null_workflow_filter_roundtrips() {
    let store = get_store().await;
    let alice = create_user(&store, "alice").await;
    let bob = create_user(&store, "bob").await;
    let now = Utc::now();

    let created = store
        .create_delegation(NewApprovalDelegation {
            from_user_id: alice,
            to_user_id: bob,
            valid_from: now,
            valid_until: now + TimeDelta::days(1),
            workflow_filter: None,
        })
        .await
        .expect("create");

    let found = store
        .find_delegation_by_id(created.id)
        .await
        .expect("find")
        .expect("some");
    assert!(found.workflow_filter.is_none());
    assert!(found.matches_workflow("anything"));
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn only_the_currently_valid_row_is_listed() {
    let store = get_store().await;
    let alice = create_user(&store, "alice").await;
    let bob = create_user(&store, "bob").await;
    let now = Utc::now();

    store
        .create_delegation(NewApprovalDelegation {
            from_user_id: alice,
            to_user_id: bob,
            valid_from: now - TimeDelta::days(10),
            valid_until: now - TimeDelta::days(3),
            workflow_filter: None,
        })
        .await
        .expect("expired");
    store
        .create_delegation(NewApprovalDelegation {
            from_user_id: alice,
            to_user_id: bob,
            valid_from: now + TimeDelta::days(3),
            valid_until: now + TimeDelta::days(10),
            workflow_filter: None,
        })
        .await
        .expect("future");
    let current = store
        .create_delegation(NewApprovalDelegation {
            from_user_id: alice,
            to_user_id: bob,
            valid_from: now - TimeDelta::hours(1),
            valid_until: now + TimeDelta::hours(1),
            workflow_filter: None,
        })
        .await
        .expect("current");

    let listed = store
        .list_active_delegations(
            DelegationFilter {
                to_user_id: Some(bob),
                ..DelegationFilter::default()
            },
            1,
            100,
        )
        .await
        .expect("list")
        .items;

    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, current.id);
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn the_optional_filters_narrow_the_result() {
    let store = get_store().await;
    let alice = create_user(&store, "alice").await;
    let bob = create_user(&store, "bob").await;
    let carol = create_user(&store, "carol").await;
    let now = Utc::now();

    let window = |from: Uuid, to: Uuid| NewApprovalDelegation {
        from_user_id: from,
        to_user_id: to,
        valid_from: now - TimeDelta::hours(1),
        valid_until: now + TimeDelta::hours(1),
        workflow_filter: None,
    };

    let alice_to_bob = store
        .create_delegation(window(alice, bob))
        .await
        .expect("alice -> bob");
    let carol_to_bob = store
        .create_delegation(window(carol, bob))
        .await
        .expect("carol -> bob");

    let by_delegate = store
        .list_active_delegations(
            DelegationFilter {
                to_user_id: Some(bob),
                ..DelegationFilter::default()
            },
            1,
            100,
        )
        .await
        .expect("list by delegate")
        .items;
    let ids: Vec<_> = by_delegate.iter().map(|d| d.id).collect();
    assert!(ids.contains(&alice_to_bob.id));
    assert!(ids.contains(&carol_to_bob.id));

    let by_delegator = store
        .list_active_delegations(
            DelegationFilter {
                from_user_id: Some(alice),
                to_user_id: Some(bob),
                ..DelegationFilter::default()
            },
            1,
            100,
        )
        .await
        .expect("list by both")
        .items;
    assert_eq!(by_delegator.len(), 1);
    assert_eq!(by_delegator[0].id, alice_to_bob.id);
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn deleting_twice_reports_not_found() {
    let store = get_store().await;
    let alice = create_user(&store, "alice").await;
    let bob = create_user(&store, "bob").await;
    let now = Utc::now();

    let created = store
        .create_delegation(NewApprovalDelegation {
            from_user_id: alice,
            to_user_id: bob,
            valid_from: now,
            valid_until: now + TimeDelta::days(1),
            workflow_filter: None,
        })
        .await
        .expect("create");

    store.delete_delegation(created.id).await.expect("delete");

    let err = store.delete_delegation(created.id).await.unwrap_err();
    assert!(matches!(err, StoreError::DelegationNotFound(_)));
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn involving_filter_and_pagination_work_in_sql() {
    let store = get_store().await;
    let alice = create_user(&store, "alice").await;
    let bob = create_user(&store, "bob").await;
    let carol = create_user(&store, "carol").await;
    let now = Utc::now();

    let window = |from: Uuid, to: Uuid| NewApprovalDelegation {
        from_user_id: from,
        to_user_id: to,
        valid_from: now - TimeDelta::hours(1),
        valid_until: now + TimeDelta::hours(1),
        workflow_filter: None,
    };

    let granted = store
        .create_delegation(window(alice, bob))
        .await
        .expect("alice -> bob");
    let received = store
        .create_delegation(window(carol, alice))
        .await
        .expect("carol -> alice");
    store
        .create_delegation(window(bob, carol))
        .await
        .expect("bob -> carol");

    let involving = DelegationFilter {
        involving_user_id: Some(alice),
        ..DelegationFilter::default()
    };

    let first = store
        .list_active_delegations(involving.clone(), 1, 1)
        .await
        .expect("page 1");
    assert_eq!(first.total, 2);
    assert_eq!(first.items.len(), 1);
    assert_eq!(first.items[0].id, received.id, "newest first");

    let second = store
        .list_active_delegations(involving, 2, 1)
        .await
        .expect("page 2");
    assert_eq!(second.items.len(), 1);
    assert_eq!(second.items[0].id, granted.id);
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn find_active_delegation_applies_pair_window_and_glob() {
    let store = get_store().await;
    let alice = create_user(&store, "alice").await;
    let bob = create_user(&store, "bob").await;
    let now = Utc::now();

    store
        .create_delegation(NewApprovalDelegation {
            from_user_id: alice,
            to_user_id: bob,
            valid_from: now - TimeDelta::days(2),
            valid_until: now - TimeDelta::days(1),
            workflow_filter: None,
        })
        .await
        .expect("expired");
    let deploy = store
        .create_delegation(NewApprovalDelegation {
            from_user_id: alice,
            to_user_id: bob,
            valid_from: now - TimeDelta::hours(1),
            valid_until: now + TimeDelta::hours(1),
            workflow_filter: Some("deploy-*".to_string()),
        })
        .await
        .expect("deploy");

    let found = store
        .find_active_delegation(alice, bob, "deploy-api")
        .await
        .expect("find");
    assert_eq!(found.map(|d| d.id), Some(deploy.id));

    let unmatched = store
        .find_active_delegation(alice, bob, "billing")
        .await
        .expect("find");
    assert!(unmatched.is_none());

    let reversed = store
        .find_active_delegation(bob, alice, "deploy-api")
        .await
        .expect("find");
    assert!(reversed.is_none());
}
