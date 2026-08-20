#![cfg(feature = "store-postgres")]

//! Integration tests for run authorship (`created_by`) against PostgreSQL.
//!
//! Covers the two nullable columns, the `LEFT JOIN` label resolution, and the
//! author filter. Requires a live database:
//!
//! ```sh
//! DATABASE_URL=postgres://... cargo test -p ironflow-store \
//!     --features store-postgres --test postgres_run_created_by -- --ignored
//! ```

use std::collections::HashMap;
use std::env::var;

use ironflow_store::api_key_store::ApiKeyStore;
use ironflow_store::entities::{ApiKeyScope, ApiKeyUpdate, NewApiKey, NewUser};
use ironflow_store::postgres::PostgresStore;
use ironflow_store::prelude::*;
use ironflow_store::store::RunStore;
use ironflow_store::user_store::UserStore;
use serde_json::json;
use uuid::Uuid;

async fn get_store() -> PostgresStore {
    let url = var("DATABASE_URL").expect("DATABASE_URL must be set");
    PostgresStore::new(&url)
        .await
        .expect("failed to connect to PostgreSQL")
}

fn new_run(name: &str, created_by: Option<RunActor>) -> NewRun {
    NewRun {
        workflow_name: name.to_string(),
        trigger: TriggerKind::Api,
        payload: json!({}),
        max_retries: 0,
        handler_version: None,
        labels: HashMap::new(),
        scheduled_at: None,
        created_by,
        idempotency_key: None,
        max_cost_usd: None,
    }
}

/// Create a user with a unique username so repeated runs do not collide.
async fn seed_user(store: &PostgresStore) -> (Uuid, String) {
    let suffix = Uuid::now_v7().simple().to_string();
    let username = format!("author-{suffix}");
    let user = store
        .create_user(NewUser {
            email: format!("{username}@example.com"),
            username: username.clone(),
            password_hash: "hash".to_string(),
            is_admin: Some(false),
        })
        .await
        .expect("create user");
    (user.id, username)
}

async fn seed_api_key(store: &PostgresStore, user_id: Uuid, name: &str) -> Uuid {
    store
        .create_api_key(NewApiKey {
            user_id,
            name: name.to_string(),
            key_hash: "hash".to_string(),
            key_prefix: format!("irfl_{}", &Uuid::now_v7().simple().to_string()[..8]),
            scopes: vec![ApiKeyScope::RunsWrite],
            expires_at: None,
            rate_limit_override: None,
        })
        .await
        .expect("create api key")
        .id
}

#[tokio::test]
#[ignore]
async fn created_by_is_null_without_actor() {
    let store = get_store().await;
    let created = store
        .create_run(new_run("created-by-none", None))
        .await
        .unwrap()
        .into_run();

    let run = store.get_run(created.id).await.unwrap().unwrap();
    assert!(run.created_by.is_none());
    assert!(run.created_by_label.is_none());
}

#[tokio::test]
#[ignore]
async fn created_by_user_round_trips_with_username_label() {
    let store = get_store().await;
    let (user_id, username) = seed_user(&store).await;

    let created = store
        .create_run(new_run("created-by-user", Some(RunActor::User { user_id })))
        .await
        .unwrap()
        .into_run();
    assert_eq!(created.created_by, Some(RunActor::User { user_id }));
    assert_eq!(created.created_by_label.as_deref(), Some(username.as_str()));

    let run = store.get_run(created.id).await.unwrap().unwrap();
    assert_eq!(run.created_by, Some(RunActor::User { user_id }));
    assert_eq!(run.created_by_label.as_deref(), Some(username.as_str()));
}

#[tokio::test]
#[ignore]
async fn created_by_api_key_label_combines_key_and_owner() {
    let store = get_store().await;
    let (user_id, username) = seed_user(&store).await;
    let api_key_id = seed_api_key(&store, user_id, "ci-deploy").await;

    let created = store
        .create_run(new_run(
            "created-by-key",
            Some(RunActor::ApiKey {
                api_key_id,
                user_id,
            }),
        ))
        .await
        .unwrap()
        .into_run();

    let run = store.get_run(created.id).await.unwrap().unwrap();
    assert_eq!(
        run.created_by,
        Some(RunActor::ApiKey {
            api_key_id,
            user_id
        })
    );
    assert_eq!(
        run.created_by_label.as_deref(),
        Some(format!("ci-deploy ({username})").as_str())
    );
}

#[tokio::test]
#[ignore]
async fn label_follows_api_key_rename() {
    let store = get_store().await;
    let (user_id, username) = seed_user(&store).await;
    let api_key_id = seed_api_key(&store, user_id, "ci-deploy").await;
    let created = store
        .create_run(new_run(
            "created-by-key-rename",
            Some(RunActor::ApiKey {
                api_key_id,
                user_id,
            }),
        ))
        .await
        .unwrap()
        .into_run();

    store
        .update_api_key(
            api_key_id,
            ApiKeyUpdate {
                name: Some("ci-release".to_string()),
                ..ApiKeyUpdate::default()
            },
        )
        .await
        .unwrap();

    let run = store.get_run(created.id).await.unwrap().unwrap();
    assert_eq!(
        run.created_by_label.as_deref(),
        Some(format!("ci-release ({username})").as_str())
    );
}

#[tokio::test]
#[ignore]
async fn authorship_survives_user_deletion() {
    let store = get_store().await;
    let (user_id, _) = seed_user(&store).await;
    let created = store
        .create_run(new_run(
            "created-by-deleted-user",
            Some(RunActor::User { user_id }),
        ))
        .await
        .unwrap()
        .into_run();

    store.delete_user(user_id).await.unwrap();

    let run = store.get_run(created.id).await.unwrap().unwrap();
    assert_eq!(run.created_by, Some(RunActor::User { user_id }));
    assert!(
        run.created_by_label.is_none(),
        "label cannot be resolved once the user is gone"
    );
}

#[tokio::test]
#[ignore]
async fn list_runs_filters_by_author() {
    let store = get_store().await;
    let (alice, alice_name) = seed_user(&store).await;
    let (bob, _) = seed_user(&store).await;

    store
        .create_run(new_run(
            "filter-alice",
            Some(RunActor::User { user_id: alice }),
        ))
        .await
        .unwrap()
        .into_run();
    store
        .create_run(new_run("filter-bob", Some(RunActor::User { user_id: bob })))
        .await
        .unwrap()
        .into_run();

    let page = store
        .list_runs(
            RunFilter {
                created_by_user_id: Some(alice),
                ..RunFilter::default()
            },
            1,
            100,
        )
        .await
        .unwrap();

    assert_eq!(page.total, 1);
    assert_eq!(
        page.items[0].created_by_label.as_deref(),
        Some(alice_name.as_str())
    );
}

#[tokio::test]
#[ignore]
async fn list_runs_author_filter_matches_runs_from_the_users_api_keys() {
    let store = get_store().await;
    let (alice, _) = seed_user(&store).await;
    let api_key_id = seed_api_key(&store, alice, "ci-deploy").await;

    store
        .create_run(new_run(
            "filter-alice-key",
            Some(RunActor::ApiKey {
                api_key_id,
                user_id: alice,
            }),
        ))
        .await
        .unwrap()
        .into_run();

    let page = store
        .list_runs(
            RunFilter {
                created_by_user_id: Some(alice),
                ..RunFilter::default()
            },
            1,
            100,
        )
        .await
        .unwrap();

    assert_eq!(page.total, 1);
}

#[tokio::test]
#[ignore]
async fn author_filter_combines_with_other_filters() {
    let store = get_store().await;
    let (alice, _) = seed_user(&store).await;
    let workflow = format!("combined-{}", Uuid::now_v7().simple());

    store
        .create_run(new_run(&workflow, Some(RunActor::User { user_id: alice })))
        .await
        .unwrap()
        .into_run();

    let page = store
        .list_runs(
            RunFilter {
                workflow_name: Some(workflow.clone()),
                status: Some(RunStatus::Pending),
                created_by_user_id: Some(alice),
                ..RunFilter::default()
            },
            1,
            100,
        )
        .await
        .unwrap();

    assert_eq!(page.total, 1);
    assert_eq!(page.items[0].workflow_name, workflow);
}

#[tokio::test]
#[ignore]
async fn unknown_author_returns_empty_page() {
    let store = get_store().await;

    let page = store
        .list_runs(
            RunFilter {
                created_by_user_id: Some(Uuid::now_v7()),
                ..RunFilter::default()
            },
            1,
            100,
        )
        .await
        .unwrap();

    assert_eq!(page.total, 0);
    assert!(page.items.is_empty());
}

#[tokio::test]
#[ignore]
async fn get_stats_honours_the_author_filter() {
    let store = get_store().await;
    let (alice, _) = seed_user(&store).await;
    let (bob, _) = seed_user(&store).await;
    let workflow = format!("stats-{}", Uuid::now_v7().simple());

    store
        .create_run(new_run(&workflow, Some(RunActor::User { user_id: alice })))
        .await
        .unwrap()
        .into_run();
    store
        .create_run(new_run(&workflow, Some(RunActor::User { user_id: bob })))
        .await
        .unwrap()
        .into_run();

    let stats = store
        .get_stats(RunFilter {
            workflow_name: Some(workflow),
            created_by_user_id: Some(alice),
            ..RunFilter::default()
        })
        .await
        .unwrap();

    assert_eq!(stats.total_runs, 1);
}

#[tokio::test]
#[ignore]
async fn pick_next_pending_resolves_author_label() {
    let store = get_store().await;
    let (user_id, username) = seed_user(&store).await;
    let created = store
        .create_run(new_run(
            "pick-next-author",
            Some(RunActor::User { user_id }),
        ))
        .await
        .unwrap()
        .into_run();

    // The pending queue is shared with every other suite hitting this database,
    // so drain it until our own run comes up instead of assuming it is first.
    let mut drained = 0;
    let picked = loop {
        let Some(run) = store.pick_next_pending(None).await.unwrap() else {
            panic!("pending queue drained ({drained} runs) without yielding the seeded run");
        };
        drained += 1;
        if run.id == created.id {
            break run;
        }
        assert!(
            drained < 1000,
            "picked {drained} runs without reaching the seeded one"
        );
    };

    assert_eq!(picked.created_by, Some(RunActor::User { user_id }));
    assert_eq!(picked.created_by_label.as_deref(), Some(username.as_str()));
}
