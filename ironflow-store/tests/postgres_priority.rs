#![cfg(feature = "store-postgres")]

//! Integration tests for run priority on the PostgreSQL store.
//!
//! `pick_next_pending` orders its candidates with `priority DESC, created_at
//! ASC`, and the column carries a CHECK constraint. Only a real database
//! exercises them.
//!
//! They need a live database and are ignored by default:
//!
//! ```sh
//! DATABASE_URL=postgres://... cargo test -p ironflow-store \
//!     --features store-postgres --test postgres_priority -- --ignored
//! ```

use std::collections::HashMap;
use std::env::var;

use chrono::{TimeDelta, Utc};
use ironflow_store::postgres::PostgresStore;
use ironflow_store::prelude::*;
use ironflow_store::store::RunStore;
use serde_json::json;
use tokio::sync::Mutex;
use uuid::Uuid;

/// Serialises the tests of this file. They pick from the global queue, so two
/// of them running at once would steal each other's runs.
static SERIAL: Mutex<()> = Mutex::const_new(());

async fn get_store() -> PostgresStore {
    let url = var("DATABASE_URL").expect("DATABASE_URL must be set");
    PostgresStore::new(&url)
        .await
        .expect("failed to connect to PostgreSQL")
}

fn new_run(priority: i16) -> NewRun {
    NewRun {
        workflow_name: "priority".to_string(),
        trigger: TriggerKind::Manual,
        payload: json!({}),
        max_retries: 0,
        handler_version: None,
        labels: HashMap::new(),
        scheduled_at: None,
        created_by: None,
        idempotency_key: None,
        concurrency_key: None,
        priority,
        concurrency_limits: Vec::new(),
        max_cost_usd: None,
    }
}

async fn create(store: &PostgresStore, req: NewRun) -> Run {
    store.create_run(req).await.unwrap().into_run()
}

async fn pick_id(store: &PostgresStore) -> Option<Uuid> {
    store.pick_next_pending(None).await.unwrap().map(|r| r.id)
}

/// Drain every pickable run so a test only sees the runs it created.
async fn drain_pending(store: &PostgresStore) {
    while store.pick_next_pending(None).await.unwrap().is_some() {}
}

/// Move the given runs to a terminal state so they do not stay in the queue
/// for the next test.
async fn finish(store: &PostgresStore, ids: &[Uuid]) {
    for id in ids {
        let run = store.get_run(*id).await.unwrap().unwrap();
        let target = match run.status.state {
            RunStatus::Pending | RunStatus::Sleeping => RunStatus::Cancelled,
            RunStatus::Running => RunStatus::Completed,
            _ => continue,
        };
        store.update_run_status(*id, target).await.unwrap();
    }
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn priority_orders_the_queue_then_fifo() {
    let _serial = SERIAL.lock().await;
    let store = get_store().await;
    drain_pending(&store).await;

    let default_old = create(&store, new_run(0)).await;
    let background = create(&store, new_run(-10)).await;
    let urgent = create(&store, new_run(50)).await;
    let default_young = create(&store, new_run(0)).await;

    let mut order = Vec::new();
    for _ in 0..4 {
        order.push(pick_id(&store).await.expect("a run is due"));
    }
    assert_eq!(
        order,
        vec![urgent.id, default_old.id, default_young.id, background.id]
    );
    assert_eq!(pick_id(&store).await, None);

    finish(&store, &order).await;
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn priority_does_not_bypass_scheduled_at() {
    let _serial = SERIAL.lock().await;
    let store = get_store().await;
    drain_pending(&store).await;

    let deferred = create(
        &store,
        NewRun {
            scheduled_at: Some(Utc::now() + TimeDelta::seconds(3600)),
            ..new_run(100)
        },
    )
    .await;
    let due = create(&store, new_run(-100)).await;

    assert_eq!(pick_id(&store).await, Some(due.id));
    assert_eq!(pick_id(&store).await, None);

    finish(&store, &[deferred.id, due.id]).await;
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn priority_round_trips_and_defaults_to_zero() {
    let _serial = SERIAL.lock().await;
    let store = get_store().await;

    let default = create(&store, new_run(0)).await;
    let urgent = create(&store, new_run(MAX_PRIORITY)).await;
    let background = create(&store, new_run(MIN_PRIORITY)).await;

    assert_eq!(
        store.get_run(default.id).await.unwrap().unwrap().priority,
        0
    );
    assert_eq!(
        store.get_run(urgent.id).await.unwrap().unwrap().priority,
        MAX_PRIORITY
    );
    assert_eq!(
        store
            .get_run(background.id)
            .await
            .unwrap()
            .unwrap()
            .priority,
        MIN_PRIORITY
    );

    finish(&store, &[default.id, urgent.id, background.id]).await;
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn priority_check_constraint_rejects_out_of_range() {
    let _serial = SERIAL.lock().await;
    let store = get_store().await;

    for priority in [MAX_PRIORITY + 1, MIN_PRIORITY - 1] {
        let err = store.create_run(new_run(priority)).await.unwrap_err();
        assert!(matches!(err, StoreError::Database(_)), "{err:?}");
    }
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn priority_filter_lists_exact_matches() {
    let _serial = SERIAL.lock().await;
    let store = get_store().await;
    // A priority no other test of this file uses, so the count is exact.
    let marked = create(&store, new_run(-37)).await;
    let other = create(&store, new_run(37)).await;

    let page = store
        .list_runs(
            RunFilter {
                priority: Some(-37),
                ..RunFilter::default()
            },
            1,
            100,
        )
        .await
        .unwrap();
    assert!(page.items.iter().any(|r| r.id == marked.id));
    assert!(page.items.iter().all(|r| r.priority == -37));

    finish(&store, &[marked.id, other.id]).await;
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn priority_schedule_firing_carries_the_schedule_priority() {
    let _serial = SERIAL.lock().await;
    let store = get_store().await;

    let schedule = store
        .create_schedule(NewSchedule {
            workflow_name: format!("priority-schedule-{}", Uuid::now_v7()),
            cron_expression: "0 0 * * * *".to_string(),
            inputs: json!({}),
            source: ScheduleSource::Api,
            priority: 25,
            created_by_user_id: None,
            next_trigger_at: Some(Utc::now() - TimeDelta::seconds(60)),
        })
        .await
        .unwrap();
    assert_eq!(schedule.priority, 25);

    let occurrence = schedule.next_trigger_at.expect("due");
    let firing = store
        .fire_due_schedule(
            schedule.id,
            occurrence,
            ScheduleNext::At(Utc::now() + TimeDelta::seconds(3600)),
        )
        .await
        .unwrap()
        .expect("due occurrence fires");
    let run_id = firing.run.run().id;
    assert_eq!(firing.run.run().priority, 25);

    let updated = store
        .update_schedule(
            schedule.id,
            ScheduleUpdate {
                priority: Some(-5),
                ..ScheduleUpdate::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.priority, -5);

    finish(&store, &[run_id]).await;
    store.delete_schedule(schedule.id).await.unwrap();
}
