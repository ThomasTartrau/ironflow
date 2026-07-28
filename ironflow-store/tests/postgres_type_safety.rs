#![cfg(feature = "store-postgres")]

//! Integration tests for PostgreSQL NUMERIC type safety.
//! Verifies that NUMERIC(12,6) columns are correctly deserialized as `rust_decimal::Decimal`.

use std::collections::HashMap;

use ironflow_store::prelude::*;
use ironflow_store::store::RunStore;
use rust_decimal::Decimal;
use serde_json::json;
use uuid::Uuid;

fn new_run(name: &str) -> NewRun {
    NewRun {
        created_by: None,
        workflow_name: name.to_string(),
        trigger: TriggerKind::Manual,
        payload: json!({}),
        max_retries: 3,
        handler_version: None,
        labels: HashMap::new(),
        scheduled_at: None,
        idempotency_key: None,
        max_cost_usd: None,
    }
}

fn new_step(run_id: Uuid, name: &str, position: u32) -> NewStep {
    NewStep {
        run_id,
        name: name.to_string(),
        kind: StepKind::Shell,
        position,
        input: None,
    }
}

async fn get_store() -> ironflow_store::postgres::PostgresStore {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    ironflow_store::postgres::PostgresStore::new(&url)
        .await
        .expect("failed to connect to PostgreSQL")
}

// ─── Run Cost Tests ─────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn cost_usd_zero_on_new_run() {
    let store = get_store().await;
    let created = store
        .create_run(new_run("test-workflow"))
        .await
        .unwrap()
        .into_run();

    let retrieved = store.get_run(created.id).await.unwrap();
    assert!(retrieved.is_some());

    let run = retrieved.unwrap();
    assert_eq!(run.cost_usd, Decimal::ZERO, "New run should have zero cost");
}

#[tokio::test]
#[ignore]
async fn cost_usd_updated_on_run() {
    let store = get_store().await;
    let created = store
        .create_run(new_run("test-workflow"))
        .await
        .unwrap()
        .into_run();

    // Update run with cost: 12.3456
    let cost_value = Decimal::new(123456, 4);
    store
        .update_run(
            created.id,
            RunUpdate {
                cost_usd: Some(cost_value),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    // Retrieve and verify
    let retrieved = store.get_run(created.id).await.unwrap();
    assert!(retrieved.is_some());

    let run = retrieved.unwrap();
    assert_eq!(
        run.cost_usd, cost_value,
        "Run cost should match updated value"
    );
}

#[tokio::test]
#[ignore]
async fn cost_usd_max_precision() {
    let store = get_store().await;
    let created = store
        .create_run(new_run("test-workflow"))
        .await
        .unwrap()
        .into_run();

    // Update run with max NUMERIC(12,6) value: 999999.999999
    let cost_value = Decimal::new(999999999999, 6);
    store
        .update_run(
            created.id,
            RunUpdate {
                cost_usd: Some(cost_value),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    // Retrieve and verify
    let retrieved = store.get_run(created.id).await.unwrap();
    assert!(retrieved.is_some());

    let run = retrieved.unwrap();
    assert_eq!(
        run.cost_usd, cost_value,
        "Run cost should support max NUMERIC(12,6) precision"
    );
}

// ─── Step Cost Tests ────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn cost_usd_zero_on_new_step() {
    let store = get_store().await;
    let run = store
        .create_run(new_run("test-workflow"))
        .await
        .unwrap()
        .into_run();

    // Transition run to Running state for step operations
    store
        .update_run_status(run.id, RunStatus::Running)
        .await
        .unwrap();

    store
        .create_step(new_step(run.id, "step-1", 1))
        .await
        .unwrap();

    // List steps and verify cost is zero
    let steps = store.list_steps(run.id).await.unwrap();

    assert_eq!(steps.len(), 1);
    let retrieved_step = &steps[0];
    assert_eq!(
        retrieved_step.cost_usd,
        Decimal::ZERO,
        "New step should have zero cost"
    );
}

#[tokio::test]
#[ignore]
async fn cost_usd_updated_on_step() {
    let store = get_store().await;
    let run = store
        .create_run(new_run("test-workflow"))
        .await
        .unwrap()
        .into_run();

    // Transition run to Running state for step operations
    store
        .update_run_status(run.id, RunStatus::Running)
        .await
        .unwrap();

    let step = store
        .create_step(new_step(run.id, "step-1", 1))
        .await
        .unwrap();

    // Transition step to Running state for update
    store
        .update_step(
            step.id,
            StepUpdate {
                status: Some(StepStatus::Running),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    // Update step with cost: 5.6789
    let cost_value = Decimal::new(56789, 4);
    store
        .update_step(
            step.id,
            StepUpdate {
                cost_usd: Some(cost_value),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    // List steps and verify cost matches
    let steps = store.list_steps(run.id).await.unwrap();

    assert_eq!(steps.len(), 1);
    let retrieved_step = &steps[0];
    assert_eq!(
        retrieved_step.cost_usd, cost_value,
        "Step cost should match updated value"
    );
}

// ─── Stats Tests ────────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn get_stats_total_cost_with_decimal() {
    let store = get_store().await;

    // Create first run with cost
    let run1 = store
        .create_run(new_run("workflow-1"))
        .await
        .unwrap()
        .into_run();
    let cost1 = Decimal::new(100000, 4); // 10.0000
    store
        .update_run(
            run1.id,
            RunUpdate {
                cost_usd: Some(cost1),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    // Create second run with different cost
    let run2 = store
        .create_run(new_run("workflow-1"))
        .await
        .unwrap()
        .into_run();
    let cost2 = Decimal::new(250000, 4); // 25.0000
    store
        .update_run(
            run2.id,
            RunUpdate {
                cost_usd: Some(cost2),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    // Get stats (covers all runs in the database)
    let stats = store.get_stats(RunFilter::default()).await.unwrap();

    // Verify total cost includes at least the sum of our two runs
    let our_total = cost1 + cost2;
    assert!(
        stats.total_cost_usd >= our_total,
        "Total cost ({}) should be >= sum of our runs ({})",
        stats.total_cost_usd,
        our_total
    );
    assert!(stats.total_runs >= 2, "Should have at least 2 runs");
}
