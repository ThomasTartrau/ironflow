#![cfg(feature = "store-postgres")]

//! Integration tests for the run `output` column on the PostgreSQL store.
//!
//! They need a live database and are ignored by default:
//!
//! ```sh
//! DATABASE_URL=postgres://... cargo test -p ironflow-store \
//!     --features store-postgres --test postgres_run_output -- --ignored
//! ```

use std::collections::HashMap;
use std::env::var;

use ironflow_store::postgres::PostgresStore;
use ironflow_store::prelude::*;
use ironflow_store::store::RunStore;
use serde_json::{Value, json};

async fn get_store() -> PostgresStore {
    let url = var("DATABASE_URL").expect("DATABASE_URL must be set");
    PostgresStore::new(&url)
        .await
        .expect("failed to connect to PostgreSQL")
}

fn new_run() -> NewRun {
    NewRun {
        workflow_name: "review".to_string(),
        trigger: TriggerKind::Manual,
        payload: json!({}),
        max_retries: 0,
        handler_version: None,
        labels: HashMap::new(),
        scheduled_at: None,
        created_by: None,
        idempotency_key: None,
        concurrency_key: None,
        max_cost_usd: None,
    }
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn postgres_run_output_round_trip() {
    let store = get_store().await;
    let run = store.create_run(new_run()).await.unwrap().into_run();
    assert!(run.output.is_none());

    let output = json!({"verdict": "approved", "findings": [1, 2]});
    store
        .update_run(
            run.id,
            RunUpdate {
                output: Some(output.clone()),
                ..RunUpdate::default()
            },
        )
        .await
        .unwrap();

    let fetched = store.get_run(run.id).await.unwrap().unwrap();
    assert_eq!(fetched.output, Some(output.clone()));

    // An update without an output leaves the stored one untouched.
    store
        .update_run(
            run.id,
            RunUpdate {
                error: Some("boom".to_string()),
                ..RunUpdate::default()
            },
        )
        .await
        .unwrap();
    let fetched = store.get_run(run.id).await.unwrap().unwrap();
    assert_eq!(fetched.output, Some(output));
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn postgres_run_output_json_null_is_kept_apart_from_no_output() {
    let store = get_store().await;
    let run = store.create_run(new_run()).await.unwrap().into_run();

    store
        .update_run(
            run.id,
            RunUpdate {
                output: Some(Value::Null),
                ..RunUpdate::default()
            },
        )
        .await
        .unwrap();

    let fetched = store.get_run(run.id).await.unwrap().unwrap();
    assert_eq!(fetched.output, Some(Value::Null));
}
