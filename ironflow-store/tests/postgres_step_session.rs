#![cfg(feature = "store-postgres")]

//! Integration tests for the `session_id` of a step on the PostgreSQL store.
//!
//! The engine records the Claude Code session of an agent step before the
//! agent launches and reads it back when it resumes an interrupted step, so
//! the column must round-trip through `update_step`, `get_step` and
//! `list_steps`, and survive later partial updates.
//!
//! They need a live database and are ignored by default:
//!
//! ```sh
//! DATABASE_URL=postgres://... cargo test -p ironflow-store \
//!     --features store-postgres --test postgres_step_session -- --ignored
//! ```

use std::collections::HashMap;
use std::env::var;

use ironflow_store::entities::{
    NewRun, NewStep, StepKind, StepStatus, StepUpdate, TriggerKind, step_trace_id,
};
use ironflow_store::postgres::PostgresStore;
use ironflow_store::store::RunStore;
use serde_json::json;

const SESSION_ID: &str = "0192f0c1-7d2e-7a4b-9c3d-1e2f3a4b5c6d";

async fn get_store() -> PostgresStore {
    let url = var("DATABASE_URL").expect("DATABASE_URL must be set");
    PostgresStore::new(&url)
        .await
        .expect("failed to connect to PostgreSQL")
}

fn new_run() -> NewRun {
    NewRun {
        workflow_name: "session".to_string(),
        trigger: TriggerKind::Manual,
        payload: json!({}),
        max_retries: 0,
        handler_version: None,
        labels: HashMap::new(),
        scheduled_at: None,
        created_by: None,
        idempotency_key: None,
        concurrency_key: None,
        concurrency_limits: Vec::new(),
        max_cost_usd: None,
    }
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn step_session_id_round_trips_and_survives_later_updates() {
    let store = get_store().await;
    let run = store.create_run(new_run()).await.unwrap().into_run();
    let step = store
        .create_step(NewStep {
            run_id: run.id,
            trace_id: step_trace_id(run.id, "review", 0),
            name: "review".to_string(),
            kind: StepKind::Agent,
            position: 0,
            input: None,
            is_error_handler: false,
        })
        .await
        .unwrap();
    assert_eq!(step.session_id, None);

    store
        .update_step(
            step.id,
            StepUpdate {
                status: Some(StepStatus::Running),
                ..StepUpdate::default()
            },
        )
        .await
        .unwrap();
    // Recorded on the running step, before the agent launches.
    store
        .update_step(
            step.id,
            StepUpdate {
                session_id: Some(SESSION_ID.to_string()),
                ..StepUpdate::default()
            },
        )
        .await
        .unwrap();
    store
        .update_step(
            step.id,
            StepUpdate {
                status: Some(StepStatus::Completed),
                duration_ms: Some(10),
                ..StepUpdate::default()
            },
        )
        .await
        .unwrap();

    let stored = store.get_step(step.id).await.unwrap().unwrap();
    assert_eq!(stored.session_id.as_deref(), Some(SESSION_ID));

    let listed = store.list_steps(run.id).await.unwrap();
    let listed = listed
        .iter()
        .find(|s| s.id == step.id)
        .expect("step is listed");
    assert_eq!(listed.session_id.as_deref(), Some(SESSION_ID));
}
