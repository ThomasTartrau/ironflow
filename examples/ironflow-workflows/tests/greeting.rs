//! Integration tests for the `greeting` example workflow.
//!
//! Black-box: the real [`Engine`] runs the real handler, and its shell step
//! spawns a real process. The `name` input is untrusted (any member can put it
//! in a schedule and trigger it), so these tests feed it shell metacharacters
//! and check that none of them is interpreted.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::time::timeout;

use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_engine::engine::Engine;
use ironflow_engine::error::EngineError;
use ironflow_engine::executor::StepOutput;
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{RunStatus, TriggerKind};
use ironflow_store::store::Store;
use ironflow_workflows::Greeting;

/// Run the greeting workflow with `payload` and return the run status and
/// the stdout of its `greet` step.
async fn run_greeting(payload: Value) -> (RunStatus, String) {
    timeout(Duration::from_secs(10), async {
        let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
        let mut engine = Engine::new(store.clone(), Arc::new(ClaudeCodeProvider::new()));
        engine.register(Greeting).expect("register greeting");

        let result = engine
            .run_handler("greeting", TriggerKind::Manual, payload)
            .await
            .expect("run the greeting workflow");

        let steps = store.list_steps(result.run.id).await.expect("list steps");
        let greet = steps
            .iter()
            .find(|step| step.name == "greet")
            .expect("greet step recorded");

        (
            result.run.status.state,
            StepOutput::from(greet).stdout().to_string(),
        )
    })
    .await
    .expect("test timed out")
}

/// A `name` that closes the quote around the message and runs `touch`.
fn quote_breakout(marker: &Path) -> String {
    format!("x'; touch {}; echo '", marker.display())
}

#[tokio::test]
async fn a_quote_in_the_name_does_not_run_an_injected_command() {
    let dir = TempDir::new().expect("temp dir");
    let marker = dir.path().join("pwned");
    let name = quote_breakout(&marker);

    let (status, stdout) = run_greeting(json!({ "name": name })).await;

    assert_eq!(status, RunStatus::Completed);
    assert!(
        !marker.exists(),
        "the injected command ran and created {}",
        marker.display()
    );
    assert_eq!(stdout, format!("Hello, {name}!"));
}

#[tokio::test]
async fn command_substitution_in_the_name_is_printed_verbatim() {
    let dir = TempDir::new().expect("temp dir");
    let marker = dir.path().join("pwned");
    let name = format!(
        "$(touch {m}) `touch {m}` ${{HOME}} %s \\n",
        m = marker.display()
    );

    let (status, stdout) = run_greeting(json!({ "name": name })).await;

    assert_eq!(status, RunStatus::Completed);
    assert!(!marker.exists(), "a substitution in the name was executed");
    assert_eq!(stdout, format!("Hello, {name}!"));
}

#[tokio::test]
async fn the_language_repeat_and_uppercase_options_still_shape_the_greeting() {
    let (status, stdout) = run_greeting(json!({
        "name": "Ada",
        "language": "fr",
        "repeat": 2,
        "uppercase": true,
    }))
    .await;

    assert_eq!(status, RunStatus::Completed);
    assert_eq!(stdout, "BONJOUR, ADA !\nBONJOUR, ADA !");
}

#[tokio::test]
async fn a_payload_without_a_name_is_rejected() {
    let result = timeout(Duration::from_secs(10), async {
        let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
        let mut engine = Engine::new(store, Arc::new(ClaudeCodeProvider::new()));
        engine.register(Greeting).expect("register greeting");
        engine
            .run_handler("greeting", TriggerKind::Manual, json!({ "language": "fr" }))
            .await
    })
    .await
    .expect("test timed out");

    match result {
        Err(EngineError::Serialization(err)) => {
            assert!(err.to_string().contains("missing field `name`"), "{err}");
        }
        Err(other) => panic!("expected an input error, got {other:?}"),
        Ok(run) => panic!(
            "expected an input error, got a run in {:?}",
            run.run.status.state
        ),
    }
}
