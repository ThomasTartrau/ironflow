//! End-to-end test of the `hello` workflow: a real engine, an in-memory
//! store, and a record/replay provider so agent steps never spend tokens.

use std::sync::Arc;

use ironflow_core::provider::AgentProvider;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_core::providers::record_replay::RecordReplayProvider;
use ironflow_engine::engine::Engine;
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{RunStatus, StepStatus, TriggerKind};
use ironflow_store::store::Store;
use serde_json::json;

use workflows::handlers;

fn engine() -> Engine {
    let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
    // Record with IRONFLOW_RECORD=1, replay from `tests/fixtures` otherwise.
    let provider: Arc<dyn AgentProvider> = Arc::new(RecordReplayProvider::new(
        ClaudeCodeProvider::new(),
        "tests/fixtures",
    ));
    let mut engine = Engine::new(store, provider);
    for handler in handlers() {
        engine.register(handler).expect("handler names are unique");
    }
    engine
}

#[tokio::test]
async fn hello_completes_with_a_greet_step() {
    let result = engine()
        .run_handler("hello", TriggerKind::Manual, json!({"name": "Ada"}))
        .await
        .expect("run completes");

    assert_eq!(result.run.status.state, RunStatus::Completed);
    assert_eq!(result.steps.len(), 1);
    assert_eq!(result.steps[0].name, "greet");
    assert_eq!(result.steps[0].status, StepStatus::Completed);
}

#[tokio::test]
async fn hello_rejects_a_payload_without_name() {
    let err = engine()
        .run_handler("hello", TriggerKind::Manual, json!({}))
        .await
        .expect_err("payload does not match HelloInput");

    assert!(err.to_string().contains("name"), "unexpected error: {err}");
}
