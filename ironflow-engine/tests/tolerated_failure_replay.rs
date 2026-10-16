//! Non-regression tests: a step tolerated with `allow_failure` that failed must
//! be replayed on resume, with the output the handler already saw, instead of
//! being executed again.
//!
//! Drives a real [`Engine`] over a real [`InMemoryStore`] with a real
//! [`WorkflowHandler`], the way `signal.rs` does. The agent provider is a
//! test-local [`AgentProvider`] that always fails and counts its calls. Test
//! names start with `replays_a_tolerated_failure` so
//! `cargo test -p ironflow-engine replays_a_tolerated_failure` selects them.

use std::env::temp_dir;
use std::fs::{read_to_string, remove_file};
use std::path::PathBuf;
use std::process::id as process_id;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::time::{sleep, timeout};
use uuid::Uuid;

use ironflow_core::error::AgentError;
use ironflow_core::provider::{AgentConfig, AgentProvider, InvokeFuture};
use ironflow_engine::config::{AgentStepConfig, ShellConfig, StepConfig};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::Engine;
use ironflow_engine::error::EngineError;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_engine::signal::Signal;
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{RunStatus, StepStatus, TriggerKind};
use ironflow_store::store::{RunStore, Store};

/// Test timeout for bodies that wait for a resumed run.
const TEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Signal key the handlers wait on.
const KEY: &str = "abc123";

/// The signal that resumes the run.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct PipelineFinished {
    status: String,
}

impl Signal for PipelineFinished {
    const NAME: &'static str = "ci.pipeline_finished";
}

/// Provider whose every invocation fails, counting the calls.
struct FailingProvider {
    calls: AtomicU32,
}

impl AgentProvider for FailingProvider {
    fn invoke<'a>(&'a self, _config: &'a AgentConfig) -> InvokeFuture<'a> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(AgentError::ProcessFailed {
                exit_code: 1,
                stderr: "boom".into(),
            })
        })
    }
}

/// What the handler saw of the tolerated step, before and after the signal.
type Seen = Arc<Mutex<Vec<Value>>>;

fn review_config() -> AgentStepConfig {
    AgentStepConfig::new("review")
        .max_budget_usd(0.10)
        .allow_failure()
}

async fn wait_for_pipeline(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    ctx.wait_for_signal::<PipelineFinished>("wait-ci", KEY, Duration::from_secs(3600))
        .await?;
    Ok(())
}

/// A tolerated agent failure, then a signal wait, then the same output read again.
struct SequentialWorkflow {
    seen: Seen,
}

impl WorkflowHandler for SequentialWorkflow {
    fn name(&self) -> &str {
        "tolerated-sequential"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let review = ctx.agent("review", review_config()).await?;
            assert!(review.error().is_some(), "the failure is tolerated");
            self.seen.lock().expect("seen lock").push(review.output);
            wait_for_pipeline(ctx).await?;
            Ok(())
        })
    }
}

/// A wave with a tolerated agent failure and a shell step, then a signal wait.
struct WaveWorkflow {
    marker: PathBuf,
    seen: Seen,
}

impl WorkflowHandler for WaveWorkflow {
    fn name(&self) -> &str {
        "tolerated-wave"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let command = format!("echo ran >> {}", self.marker.display());
            let results = ctx
                .parallel(
                    vec![
                        ("review", StepConfig::Agent(review_config())),
                        ("record", StepConfig::Shell(ShellConfig::new(&command))),
                    ],
                    false,
                )
                .await?;
            let mut seen = self.seen.lock().expect("seen lock");
            for result in results {
                seen.push(json!({ "name": result.name, "output": result.output.output }));
            }
            drop(seen);
            wait_for_pipeline(ctx).await?;
            Ok(())
        })
    }
}

/// Removes the marker file when the test ends.
struct MarkerGuard(PathBuf);

impl Drop for MarkerGuard {
    fn drop(&mut self) {
        let _ = remove_file(&self.0);
    }
}

fn setup(
    handler: impl WorkflowHandler + 'static,
) -> (Arc<Engine>, Arc<InMemoryStore>, Arc<FailingProvider>) {
    let store = Arc::new(InMemoryStore::new());
    let provider = Arc::new(FailingProvider {
        calls: AtomicU32::new(0),
    });
    let dyn_store: Arc<dyn Store> = store.clone();
    let mut engine = Engine::new(dyn_store, provider.clone());
    engine.register(handler).expect("register handler");
    (Arc::new(engine), store, provider)
}

/// Run the handler until it sleeps on the signal, returning the run id.
async fn start_waiting(engine: &Engine, workflow: &str) -> Uuid {
    let result = engine
        .run_handler(workflow, TriggerKind::Manual, json!({}))
        .await
        .expect("handler suspends on the signal");
    assert_eq!(result.run.status.state, RunStatus::Sleeping);
    result.run.id
}

/// Deliver the signal and wait for the resumed run to end as `Warning`.
async fn resume_with_signal(engine: &Engine, store: &InMemoryStore, run_id: Uuid) {
    let delivery = engine
        .send_signal(
            &PipelineFinished {
                status: "success".to_string(),
            },
            KEY,
            None,
        )
        .await
        .expect("deliver");
    assert_eq!(delivery.resumed.len(), 1);

    timeout(TEST_TIMEOUT, async {
        loop {
            let run = store
                .get_run(run_id)
                .await
                .expect("get run")
                .expect("run exists");
            if run.status.state == RunStatus::Warning {
                return;
            }
            assert!(
                !run.status.state.is_terminal(),
                "run ended as {:?}, expected Warning",
                run.status.state
            );
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("run never reached Warning");
}

#[tokio::test]
async fn replays_a_tolerated_failure_after_a_signal() {
    let seen = Seen::default();
    let (engine, store, provider) = setup(SequentialWorkflow { seen: seen.clone() });

    let run_id = start_waiting(&engine, "tolerated-sequential").await;
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    let steps = store.list_steps(run_id).await.expect("list steps");
    let review = steps
        .iter()
        .find(|s| s.name == "review")
        .expect("review step");
    assert_eq!(review.status.state, StepStatus::Failed);
    let review_id = review.id;

    resume_with_signal(&engine, &store, run_id).await;

    assert_eq!(
        provider.calls.load(Ordering::SeqCst),
        1,
        "the tolerated failure is not executed again"
    );
    let steps = store.list_steps(run_id).await.expect("list steps");
    let reviews: Vec<_> = steps.iter().filter(|s| s.name == "review").collect();
    assert_eq!(reviews.len(), 1, "no second 'review' step");
    assert_eq!(reviews[0].id, review_id);

    let seen = seen.lock().expect("seen lock");
    assert_eq!(seen.len(), 2, "the handler ran before and after the signal");
    assert_eq!(seen[0], seen[1], "the replayed output is the original one");
    assert!(seen[0]["error"].is_string());
}

#[tokio::test]
async fn replays_a_tolerated_failure_of_a_wave() {
    let marker = temp_dir().join(format!(
        "ironflow-tolerated-wave-{}-{}",
        process_id(),
        Uuid::now_v7()
    ));
    let _guard = MarkerGuard(marker.clone());
    let seen = Seen::default();
    let (engine, store, provider) = setup(WaveWorkflow {
        marker: marker.clone(),
        seen: seen.clone(),
    });

    let run_id = start_waiting(&engine, "tolerated-wave").await;
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    assert_eq!(read_to_string(&marker).expect("marker").lines().count(), 1);

    resume_with_signal(&engine, &store, run_id).await;

    assert_eq!(
        provider.calls.load(Ordering::SeqCst),
        1,
        "the tolerated failure is not executed again"
    );
    assert_eq!(
        read_to_string(&marker).expect("marker").lines().count(),
        1,
        "the completed step of the wave is replayed too"
    );
    let steps = store.list_steps(run_id).await.expect("list steps");
    for name in ["review", "record"] {
        assert_eq!(
            steps.iter().filter(|s| s.name == name).count(),
            1,
            "one '{name}' step"
        );
    }

    let seen = seen.lock().expect("seen lock");
    assert_eq!(seen.len(), 4, "two results before and two after the signal");
    assert_eq!(seen[..2], seen[2..], "the wave results are identical");
    assert_eq!(seen[0]["name"], json!("review"));
    assert!(seen[0]["output"]["error"].is_string());
}
