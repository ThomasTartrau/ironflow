#![cfg(feature = "store-postgres")]

//! Integration tests for the `environment_id` of a step on the PostgreSQL store.
//!
//! The engine reads it back from the stored step when it replays an agent step
//! after a suspension, so the column must round-trip through `update_step`,
//! `get_step` and `list_steps`, and survive later partial updates.
//!
//! They need a live database and are ignored by default:
//!
//! ```sh
//! DATABASE_URL=postgres://... cargo test -p ironflow-store \
//!     --features store-postgres --test postgres_step_environment -- --ignored
//! ```

use std::collections::HashMap;
use std::env::var;

use ironflow_store::entities::{NewRun, NewStep, StepKind, StepUpdate, TriggerKind, step_trace_id};
use ironflow_store::postgres::PostgresStore;
use ironflow_store::store::RunStore;
use serde_json::json;

async fn get_store() -> PostgresStore {
    let url = var("DATABASE_URL").expect("DATABASE_URL must be set");
    PostgresStore::new(&url)
        .await
        .expect("failed to connect to PostgreSQL")
}

fn new_run() -> NewRun {
    NewRun {
        workflow_name: "environment".to_string(),
        trigger: TriggerKind::Manual,
        payload: json!({}),
        max_retries: 0,
        handler_version: None,
        labels: HashMap::new(),
        scheduled_at: None,
        created_by: None,
        idempotency_key: None,
        concurrency_key: None,
        priority: 0,
        concurrency_limits: Vec::new(),
        max_cost_usd: None,
        worker_tags: Vec::new(),
    }
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn step_environment_id_round_trips_and_survives_later_updates() {
    let store = get_store().await;
    let run = store.create_run(new_run()).await.unwrap().into_run();
    let step = store
        .create_step(NewStep {
            run_id: run.id,
            trace_id: step_trace_id(run.id, "prepare", 0),
            name: "prepare".to_string(),
            kind: StepKind::Agent,
            position: 0,
            input: None,
            is_error_handler: false,
        })
        .await
        .unwrap();
    assert_eq!(step.environment_id, None);

    store
        .update_step(
            step.id,
            StepUpdate {
                environment_id: Some("ironflow-env-0a1b2c".to_string()),
                ..StepUpdate::default()
            },
        )
        .await
        .unwrap();
    store
        .update_step(
            step.id,
            StepUpdate {
                duration_ms: Some(10),
                ..StepUpdate::default()
            },
        )
        .await
        .unwrap();

    let stored = store.get_step(step.id).await.unwrap().unwrap();
    assert_eq!(
        stored.environment_id.as_deref(),
        Some("ironflow-env-0a1b2c")
    );

    let listed = store.list_steps(run.id).await.unwrap();
    let listed = listed
        .iter()
        .find(|s| s.id == step.id)
        .expect("step is listed");
    assert_eq!(
        listed.environment_id.as_deref(),
        Some("ironflow-env-0a1b2c")
    );
}
