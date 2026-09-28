//! Non-regression tests: `ctx.parallel()` must replay the completed steps of a
//! wave on resume instead of launching them again, whether the whole wave or
//! only part of it completed.
//!
//! Drives a real [`Engine`] over a real [`InMemoryStore`] with a real
//! [`WorkflowHandler`], the way `ironflow-engine/tests/human_input.rs` does.

use std::env::temp_dir;
use std::fs::{read_to_string, remove_file};
use std::path::PathBuf;
use std::process::id as process_id;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;
use tokio::time::timeout;
use uuid::Uuid;

use ironflow_core::provider::AgentProvider;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_engine::config::{HumanInputConfig, ShellConfig, StepConfig};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::Engine;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{RunStatus, StepStatus, StepUpdate, TriggerKind};
use ironflow_store::store::{RunStore, Store};

/// Test timeout for bodies that touch the store.
const TEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Workflow name registered by [`ParallelWorkflow`].
const WORKFLOW: &str = "parallel-replay";

/// The typed answer the handler asks for after the wave.
#[derive(Debug, Deserialize, JsonSchema)]
struct Answers {
    answers: Vec<String>,
}

/// Runs a parallel wave of two shell steps, then asks for [`Answers`].
struct ParallelWorkflow;

impl WorkflowHandler for ParallelWorkflow {
    fn name(&self) -> &str {
        WORKFLOW
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.parallel(
                vec![
                    ("wave-a", StepConfig::Shell(ShellConfig::new("echo a"))),
                    ("wave-b", StepConfig::Shell(ShellConfig::new("echo b"))),
                ],
                true,
            )
            .await?;
            let answers = ctx
                .human_input::<Answers>("clarify", HumanInputConfig::new("Answer?"))
                .await?;
            assert_eq!(answers.answers, vec!["ok".to_string()]);
            Ok(())
        })
    }
}

/// Workflow name registered by [`PartialWaveWorkflow`].
const PARTIAL_WORKFLOW: &str = "parallel-replay-partial";

/// Runs a wave where `record` completes, appending one line to `marker` per
/// execution, and `flaky` fails with `allow_failure`, then asks for
/// [`Answers`].
struct PartialWaveWorkflow {
    marker: PathBuf,
}

impl WorkflowHandler for PartialWaveWorkflow {
    fn name(&self) -> &str {
        PARTIAL_WORKFLOW
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let record = format!("echo ran >> {}", self.marker.display());
            ctx.parallel(
                vec![
                    ("record", StepConfig::Shell(ShellConfig::new(&record))),
                    (
                        "flaky",
                        StepConfig::Shell(ShellConfig::new("exit 1").allow_failure()),
                    ),
                ],
                false,
            )
            .await?;
            let answers = ctx
                .human_input::<Answers>("clarify", HumanInputConfig::new("Answer?"))
                .await?;
            assert_eq!(answers.answers, vec!["ok".to_string()]);
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

fn provider() -> Arc<dyn AgentProvider> {
    Arc::new(ClaudeCodeProvider::new())
}

fn engine_with(store: Arc<InMemoryStore>, handler: impl WorkflowHandler + 'static) -> Engine {
    let store: Arc<dyn Store> = store;
    let mut engine = Engine::new(store, provider());
    engine.register(handler).expect("register handler");
    engine
}

/// Enqueue and execute a run the way the worker does, returning its id.
async fn start(engine: &Engine, store: &Arc<InMemoryStore>) -> Uuid {
    start_workflow(engine, store, WORKFLOW).await
}

/// Enqueue and execute a run of `workflow` the way the worker does, returning
/// its id.
async fn start_workflow(engine: &Engine, store: &Arc<InMemoryStore>, workflow: &str) -> Uuid {
    let run = engine
        .enqueue_handler(workflow, TriggerKind::Manual, json!({}), 0)
        .await
        .expect("enqueue");
    store
        .update_run_status(run.id, RunStatus::Running)
        .await
        .expect("mark running");
    engine
        .execute_handler_run(run.id)
        .await
        .expect("handler suspends on the input");
    run.id
}

#[tokio::test]
async fn parallel_replay_does_not_rerun_a_completed_wave_on_resume() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let handler = ParallelWorkflow;
        let engine = engine_with(store.clone(), handler);

        let run_id = start(&engine, &store).await;

        let steps = store.list_steps(run_id).await.expect("list steps");
        let wave_a = steps.iter().find(|s| s.name == "wave-a").expect("wave-a");
        let wave_b = steps.iter().find(|s| s.name == "wave-b").expect("wave-b");
        assert_eq!(wave_a.position, 0);
        assert_eq!(wave_b.position, 0);
        let (wave_a_id, wave_b_id) = (wave_a.id, wave_b.id);

        let input_step = steps
            .iter()
            .find(|s| s.status.state == StepStatus::AwaitingApproval)
            .expect("human input step suspended the run");
        store
            .update_step(
                input_step.id,
                StepUpdate {
                    status: Some(StepStatus::Completed),
                    output: Some(json!({"answers": ["ok"]})),
                    completed_at: Some(Utc::now()),
                    clear_approval_deadline: true,
                    ..StepUpdate::default()
                },
            )
            .await
            .expect("store the answer");
        store
            .update_run_status(run_id, RunStatus::Running)
            .await
            .expect("mark running");

        // Reproduces the worker / `ExecutionMode::Workers` resume path, not
        // `resume_run`.
        engine
            .execute_handler_run(run_id)
            .await
            .expect("resume via worker pickup");

        let steps = store.list_steps(run_id).await.expect("list steps");
        let wave_a_steps: Vec<_> = steps.iter().filter(|s| s.name == "wave-a").collect();
        let wave_b_steps: Vec<_> = steps.iter().filter(|s| s.name == "wave-b").collect();
        assert_eq!(
            wave_a_steps.len(),
            1,
            "no second 'wave-a' step must be created"
        );
        assert_eq!(
            wave_b_steps.len(),
            1,
            "no second 'wave-b' step must be created"
        );
        assert_eq!(wave_a_steps[0].id, wave_a_id);
        assert_eq!(wave_b_steps[0].id, wave_b_id);

        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::Completed);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn parallel_replay_does_not_rerun_the_completed_steps_of_a_partial_wave() {
    timeout(TEST_TIMEOUT, async {
        let marker = temp_dir().join(format!(
            "ironflow-parallel-replay-{}-{}",
            process_id(),
            Uuid::now_v7()
        ));
        let _guard = MarkerGuard(marker.clone());
        let store = Arc::new(InMemoryStore::new());
        let engine = engine_with(
            store.clone(),
            PartialWaveWorkflow {
                marker: marker.clone(),
            },
        );

        let run_id = start_workflow(&engine, &store, PARTIAL_WORKFLOW).await;

        let steps = store.list_steps(run_id).await.expect("list steps");
        let record = steps.iter().find(|s| s.name == "record").expect("record");
        assert_eq!(record.status.state, StepStatus::Completed);
        let record_id = record.id;
        let flaky = steps.iter().find(|s| s.name == "flaky").expect("flaky");
        assert_eq!(flaky.status.state, StepStatus::Failed);

        let input_step = steps
            .iter()
            .find(|s| s.status.state == StepStatus::AwaitingApproval)
            .expect("human input step suspended the run");
        store
            .update_step(
                input_step.id,
                StepUpdate {
                    status: Some(StepStatus::Completed),
                    output: Some(json!({"answers": ["ok"]})),
                    completed_at: Some(Utc::now()),
                    clear_approval_deadline: true,
                    ..StepUpdate::default()
                },
            )
            .await
            .expect("store the answer");
        store
            .update_run_status(run_id, RunStatus::Running)
            .await
            .expect("mark running");

        engine
            .execute_handler_run(run_id)
            .await
            .expect("resume via worker pickup");

        let steps = store.list_steps(run_id).await.expect("list steps");
        let record_steps: Vec<_> = steps.iter().filter(|s| s.name == "record").collect();
        assert_eq!(
            record_steps.len(),
            1,
            "the completed 'record' step must be replayed, not re-created"
        );
        assert_eq!(record_steps[0].id, record_id);
        let runs = read_to_string(&marker).expect("read marker");
        assert_eq!(
            runs.lines().count(),
            1,
            "the completed 'record' command must run exactly once"
        );

        // `flaky` still fails under `allow_failure`, so the run ends in Warning.
        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::Warning);
    })
    .await
    .expect("test timed out");
}
