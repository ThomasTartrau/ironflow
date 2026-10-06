#![cfg(feature = "store-postgres")]

//! Integration tests for schedule firing on the PostgreSQL store.
//!
//! They cover what the in-memory store cannot: the transaction that ties the
//! run creation to the schedule update, `FOR UPDATE SKIP LOCKED` between
//! concurrent firings, and the `last_error` column.
//!
//! They need a live database and are ignored by default:
//!
//! ```sh
//! DATABASE_URL=postgres://... cargo test -p ironflow-store \
//!     --features store-postgres --test postgres_schedules -- --ignored
//! ```

use chrono::{DateTime, TimeDelta, Utc};
use ironflow_store::entities::{
    NewSchedule, Schedule, ScheduleNext, ScheduleSource, ScheduleUpdate,
};
use ironflow_store::postgres::PostgresStore;
use ironflow_store::schedule_store::ScheduleStore;
use ironflow_store::store::RunStore;
use serde_json::json;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

fn database_url() -> String {
    std::env::var("DATABASE_URL").expect("DATABASE_URL must be set")
}

async fn get_store() -> PostgresStore {
    PostgresStore::new(&database_url())
        .await
        .expect("failed to connect to PostgreSQL")
}

async fn raw_pool() -> PgPool {
    PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url())
        .await
        .expect("failed to connect to PostgreSQL")
}

/// A workflow name unique to this test run: the database is shared across tests.
fn unique_workflow(label: &str) -> String {
    format!("schedule-{label}-{}", Uuid::now_v7().simple())
}

/// Create a schedule due one minute ago and return it with its occurrence.
async fn due_schedule(store: &PostgresStore, workflow: &str) -> (Schedule, DateTime<Utc>) {
    let schedule = store
        .create_schedule(NewSchedule {
            workflow_name: workflow.to_string(),
            cron_expression: "0 * * * *".to_string(),
            inputs: json!({"env": "prod"}),
            source: ScheduleSource::Api,
            created_by_user_id: None,
            next_trigger_at: Some(Utc::now() - TimeDelta::seconds(60)),
        })
        .await
        .expect("create schedule");
    // Read back the stored occurrence: PostgreSQL keeps microseconds only.
    let occurrence = schedule.next_trigger_at.expect("due schedule");
    (schedule, occurrence)
}

fn next_hour() -> ScheduleNext {
    ScheduleNext::At(Utc::now() + TimeDelta::hours(1))
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn fire_creates_run_with_occurrence_key_and_advances_next_trigger() {
    let store = get_store().await;
    let (schedule, occurrence) = due_schedule(&store, &unique_workflow("fire")).await;

    let firing = store
        .fire_due_schedule(schedule.id, occurrence, next_hour())
        .await
        .expect("fire")
        .expect("due occurrence fires");

    let key = Schedule::occurrence_key(schedule.id, occurrence);
    assert!(firing.run.is_created());
    assert_eq!(
        firing.run.run().idempotency_key.as_deref(),
        Some(key.as_str())
    );
    assert_eq!(firing.run.run().payload, json!({"env": "prod"}));
    assert!(firing.schedule.is_active());
    assert!(firing.schedule.next_trigger_at.expect("next") > Utc::now());
    assert!(firing.schedule.last_triggered_at.is_some());

    let due = store.list_due_schedules().await.expect("list due");
    assert!(due.iter().all(|s| s.id != schedule.id));
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn list_due_schedules_changes_nothing() {
    let store = get_store().await;
    let (schedule, occurrence) = due_schedule(&store, &unique_workflow("list")).await;

    for _ in 0..2 {
        let due = store.list_due_schedules().await.expect("list due");
        let listed = due
            .iter()
            .find(|s| s.id == schedule.id)
            .expect("due schedule listed");
        assert_eq!(listed.next_trigger_at, Some(occurrence));
        assert!(listed.last_triggered_at.is_none());
    }
}

/// The exact failure of #173: the run cannot be created. Nothing may be
/// written, so the schedule is still due at the same occurrence.
#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn failed_run_creation_leaves_the_schedule_due() {
    let store = get_store().await;
    let workflow = unique_workflow("boom");
    let (schedule, occurrence) = due_schedule(&store, &workflow).await;

    // A trigger that rejects this workflow's runs: a real database failure,
    // scoped to this test by its unique workflow name.
    let suffix = Uuid::now_v7().simple().to_string();
    let function = format!("ironflow.reject_run_{suffix}");
    let trigger = format!("reject_run_{suffix}");
    let pool = raw_pool().await;
    sqlx::query(&format!(
        "CREATE FUNCTION {function}() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN IF NEW.workflow_name = '{workflow}' THEN \
         RAISE EXCEPTION 'run rejected by test'; END IF; RETURN NEW; END $$"
    ))
    .execute(&pool)
    .await
    .expect("create function");
    sqlx::query(&format!(
        "CREATE TRIGGER {trigger} BEFORE INSERT ON ironflow.runs \
         FOR EACH ROW EXECUTE FUNCTION {function}()"
    ))
    .execute(&pool)
    .await
    .expect("create trigger");

    let result = store
        .fire_due_schedule(schedule.id, occurrence, next_hour())
        .await;

    sqlx::query(&format!("DROP TRIGGER {trigger} ON ironflow.runs"))
        .execute(&pool)
        .await
        .expect("drop trigger");
    sqlx::query(&format!("DROP FUNCTION {function}()"))
        .execute(&pool)
        .await
        .expect("drop function");

    let err = result.expect_err("run creation must fail");
    assert!(err.to_string().contains("run rejected by test"), "{err}");

    let stored = store
        .find_schedule_by_id(schedule.id)
        .await
        .expect("find")
        .expect("exists");
    assert!(stored.is_active());
    assert_eq!(stored.next_trigger_at, Some(occurrence));
    assert!(stored.last_triggered_at.is_none());
    let key = Schedule::occurrence_key(schedule.id, occurrence);
    assert!(
        store
            .find_run_by_idempotency_key(&key)
            .await
            .expect("find run")
            .is_none()
    );

    // Retried on the next tick, the occurrence fires.
    let firing = store
        .fire_due_schedule(schedule.id, occurrence, next_hour())
        .await
        .expect("retry")
        .expect("still due");
    assert!(firing.run.is_created());
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn concurrent_fires_of_one_occurrence_create_one_run() {
    let store = get_store().await;
    let (schedule, occurrence) = due_schedule(&store, &unique_workflow("race")).await;

    let (first, second) = tokio::join!(
        store.fire_due_schedule(schedule.id, occurrence, next_hour()),
        store.fire_due_schedule(schedule.id, occurrence, next_hour()),
    );

    let fired = [first.expect("first"), second.expect("second")]
        .into_iter()
        .flatten()
        .count();
    assert_eq!(fired, 1, "exactly one instance fires the occurrence");

    let key = Schedule::occurrence_key(schedule.id, occurrence);
    assert!(
        store
            .find_run_by_idempotency_key(&key)
            .await
            .expect("find run")
            .is_some()
    );
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn fire_with_disable_stores_last_error() {
    let store = get_store().await;
    let (schedule, occurrence) = due_schedule(&store, &unique_workflow("disable")).await;

    let firing = store
        .fire_due_schedule(
            schedule.id,
            occurrence,
            ScheduleNext::Disable {
                error: "cannot compute next trigger".to_string(),
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
    assert_eq!(
        stored.last_error.as_deref(),
        Some("cannot compute next trigger")
    );
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn fire_paused_schedule_returns_none() {
    let store = get_store().await;
    let (schedule, occurrence) = due_schedule(&store, &unique_workflow("paused")).await;
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
        .fire_due_schedule(schedule.id, occurrence, next_hour())
        .await
        .expect("fire");

    assert!(fired.is_none());
    let key = Schedule::occurrence_key(schedule.id, occurrence);
    assert!(
        store
            .find_run_by_idempotency_key(&key)
            .await
            .expect("find run")
            .is_none()
    );
}
