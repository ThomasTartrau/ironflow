#![cfg(feature = "store-postgres")]

//! Integration tests for schedule authorship against PostgreSQL.
//!
//! These tests exist because `InMemoryStore` has no foreign-key constraint, so
//! it silently accepted the `Uuid::nil()` author that handler-declared schedules
//! used to insert. On PostgreSQL that value violated
//! `schedules_created_by_user_id_fkey` and crashed the server at startup.
//!
//! Requires a live database:
//!
//! ```sh
//! DATABASE_URL=postgres://... cargo test -p ironflow-api \
//!     --features store-postgres --test postgres_schedule_sync -- --ignored
//! ```

use std::env::var;
use std::sync::Arc;

use ironflow_api::schedule_sync::sync_handler_schedules;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::Engine;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_engine::prelude::CronSchedule;
use ironflow_store::entities::{NewSchedule, NewUser, ScheduleSource};
use ironflow_store::postgres::PostgresStore;
use ironflow_store::schedule_store::ScheduleStore;
use ironflow_store::store::Store;
use ironflow_store::user_store::UserStore;
use serde_json::json;
use uuid::Uuid;

async fn get_store() -> PostgresStore {
    let url = var("DATABASE_URL").expect("DATABASE_URL must be set");
    PostgresStore::new(&url)
        .await
        .expect("failed to connect to PostgreSQL")
}

/// Create a user with a unique username so repeated runs do not collide.
async fn seed_user(store: &PostgresStore) -> Uuid {
    let suffix = Uuid::now_v7().simple().to_string();
    let username = format!("sched-author-{suffix}");
    store
        .create_user(NewUser {
            email: format!("{username}@example.com"),
            username,
            password_hash: "hash".to_string(),
            is_admin: Some(false),
        })
        .await
        .expect("create user")
        .id
}

/// A handler that declares a cron schedule, used to drive `sync_handler_schedules`.
struct NamedScheduled {
    wf_name: String,
    cron: CronSchedule,
}

impl WorkflowHandler for NamedScheduled {
    fn name(&self) -> &str {
        &self.wf_name
    }
    fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async { Ok(()) })
    }
    fn schedule(&self) -> Option<&CronSchedule> {
        Some(&self.cron)
    }
}

/// Regression: a brand-new handler-declared schedule must sync to PostgreSQL
/// without violating the `created_by_user_id` foreign key.
///
/// Before the fix, `sync_handler_schedules` inserted `Uuid::nil()`, which is not
/// a row in `iam.users`, so the INSERT failed with
/// `schedules_created_by_user_id_fkey` and the server crashed at boot.
#[tokio::test]
#[ignore]
async fn sync_creates_handler_schedule_without_fk_violation() {
    let store = get_store().await;
    let store: Arc<dyn Store> = Arc::new(store);
    let wf_name = format!("nightly-{}", Uuid::now_v7().simple());

    let provider = Arc::new(ClaudeCodeProvider::new());
    let mut engine = Engine::new(store.clone(), provider);
    engine
        .register(NamedScheduled {
            wf_name: wf_name.clone(),
            cron: CronSchedule::new("0 0 * * *").expect("valid cron"),
        })
        .expect("register");

    // The literal symptom from the issue: this call used to return the FK error.
    sync_handler_schedules(&engine, store.as_ref())
        .await
        .expect("sync must not violate the FK");

    let page = store.list_schedules(1, 1000).await.expect("list");
    let synced = page
        .items
        .iter()
        .find(|s| s.workflow_name == wf_name)
        .expect("handler schedule was persisted");
    assert_eq!(synced.source, ScheduleSource::Handler);
    assert_eq!(
        synced.created_by_user_id, None,
        "handler schedules have no human author"
    );
}

/// A schedule created with no author persists NULL and reads back as `None`.
#[tokio::test]
#[ignore]
async fn create_schedule_with_none_persists_null() {
    let store = get_store().await;
    let wf_name = format!("no-author-{}", Uuid::now_v7().simple());

    let created = store
        .create_schedule(NewSchedule {
            workflow_name: wf_name,
            cron_expression: "0 0 * * *".to_string(),
            inputs: json!({}),
            source: ScheduleSource::Handler,
            created_by_user_id: None,
            next_trigger_at: None,
        })
        .await
        .expect("create with None must not violate the FK");
    assert_eq!(created.created_by_user_id, None);

    let fetched = store
        .find_schedule_by_id(created.id)
        .await
        .expect("find")
        .expect("schedule exists");
    assert_eq!(
        fetched.created_by_user_id, None,
        "NULL author round-trips as None"
    );
}

/// A schedule created with an author round-trips the exact user UUID.
#[tokio::test]
#[ignore]
async fn create_schedule_with_some_round_trips_the_author() {
    let store = get_store().await;
    let user_id = seed_user(&store).await;
    let wf_name = format!("with-author-{}", Uuid::now_v7().simple());

    let created = store
        .create_schedule(NewSchedule {
            workflow_name: wf_name,
            cron_expression: "0 0 * * *".to_string(),
            inputs: json!({}),
            source: ScheduleSource::Api,
            created_by_user_id: Some(user_id),
            next_trigger_at: None,
        })
        .await
        .expect("create with a real author");
    assert_eq!(created.created_by_user_id, Some(user_id));

    let fetched = store
        .find_schedule_by_id(created.id)
        .await
        .expect("find")
        .expect("schedule exists");
    assert_eq!(fetched.created_by_user_id, Some(user_id));
}

/// The foreign key is still enforced: an author that is not a real user is
/// rejected. This proves the fix relaxed nullability without dropping the FK.
#[tokio::test]
#[ignore]
async fn create_schedule_with_unknown_author_is_rejected() {
    let store = get_store().await;
    let wf_name = format!("bad-author-{}", Uuid::now_v7().simple());

    let result = store
        .create_schedule(NewSchedule {
            workflow_name: wf_name,
            cron_expression: "0 0 * * *".to_string(),
            inputs: json!({}),
            source: ScheduleSource::Api,
            created_by_user_id: Some(Uuid::now_v7()),
            next_trigger_at: None,
        })
        .await;

    assert!(
        result.is_err(),
        "a non-existent author must still violate the FK"
    );
}
