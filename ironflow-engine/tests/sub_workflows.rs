//! Integration tests for typed sub-workflows.
//!
//! A child declares its input through [`TypedWorkflow`]; the parent calls
//! `ctx.workflow(&Child, ChildInput { .. })` and gets the child run id back as
//! a [`Uuid`]. Every test drives a real [`Engine`] over a real
//! [`InMemoryStore`] and real shell steps.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::time::timeout;
use uuid::Uuid;

use ironflow_core::provider::AgentProvider;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_core::providers::record_replay::RecordReplayProvider;
use ironflow_engine::config::ShellConfig;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::Engine;
use ironflow_engine::handler::{HandlerFuture, TypedWorkflow, WorkflowHandler};
use ironflow_engine::plan::{ConditionResult, PlanOptions};
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{RunFilter, RunStatus, StepKind, TriggerKind};
use ironflow_store::store::{RunStore, Store};

/// Test timeout for bodies that spawn processes.
const TEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Input of [`Greeter`].
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct GreetInput {
    name: String,
    shout: bool,
}

/// A child whose input is typed.
struct Greeter;

impl WorkflowHandler for Greeter {
    fn name(&self) -> &str {
        "greeter"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            if ctx.when("shouting", |i: &GreetInput| i.shout).await? {
                ctx.shell("greet-loud", ShellConfig::new("echo HELLO"))
                    .await?;
            } else {
                ctx.shell("greet", ShellConfig::new("echo hello")).await?;
            }
            Ok(())
        })
    }
}

impl TypedWorkflow for Greeter {
    type Input = GreetInput;
}

/// Calls [`Greeter`] and remembers the child run id it got back.
struct Host {
    child_run_id: Arc<Mutex<Option<Uuid>>>,
}

impl WorkflowHandler for Host {
    fn name(&self) -> &str {
        "host"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let child = ctx
                .workflow(
                    &Greeter,
                    GreetInput {
                        name: "Élodie".to_string(),
                        shout: true,
                    },
                )
                .await?;
            *self.child_run_id.lock().expect("lock") = Some(child.run_id());
            Ok(())
        })
    }
}

fn provider() -> Arc<dyn AgentProvider> {
    Arc::new(RecordReplayProvider::replay(
        ClaudeCodeProvider::new(),
        "/tmp/ironflow-fixtures",
    ))
}

fn engine(store: Arc<InMemoryStore>, child_run_id: Arc<Mutex<Option<Uuid>>>) -> Engine {
    let store: Arc<dyn Store> = store;
    let mut engine = Engine::new(store, provider());
    engine.register(Greeter).expect("register child");
    engine
        .register(Host { child_run_id })
        .expect("register parent");
    engine
}

#[tokio::test]
async fn a_typed_sub_workflow_runs_with_its_input_and_reports_its_run_id() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let seen = Arc::new(Mutex::new(None));
        let engine = engine(store.clone(), seen.clone());

        let result = engine
            .run_handler("host", TriggerKind::Manual, json!({}))
            .await
            .expect("parent completes");
        assert_eq!(result.run.status.state, RunStatus::Completed);

        let child = store
            .list_runs(RunFilter::default(), 1, 10)
            .await
            .expect("list runs")
            .items
            .into_iter()
            .find(|r| r.workflow_name == "greeter")
            .expect("child run exists");

        assert_eq!(*seen.lock().expect("lock"), Some(child.id));
        assert_eq!(child.payload, json!({"name": "Élodie", "shout": true}));

        let child_steps = store.list_steps(child.id).await.expect("child steps");
        assert_eq!(child_steps.len(), 1);
        assert_eq!(child_steps[0].name, "greet-loud");
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn a_planned_sub_workflow_is_expanded_with_its_typed_input_and_a_nil_run_id() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let seen = Arc::new(Mutex::new(None));
        let engine = engine(store.clone(), seen.clone());

        let plan = engine
            .plan_handler("host", json!({}), PlanOptions::default())
            .await
            .expect("plan built");

        assert!(
            !plan.truncated,
            "plan stopped: {:?}",
            plan.incomplete_reason
        );
        let names: Vec<&str> = plan.steps.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["greeter", "greet-loud"]);
        assert_eq!(plan.steps[0].kind, StepKind::Workflow);
        match plan.steps[1].condition.as_ref().expect("a condition") {
            ConditionResult::Evaluated { expression, value } => {
                assert_eq!(expression, "shouting");
                assert!(*value);
            }
            other => panic!("expected an evaluated condition, got {other:?}"),
        }

        assert_eq!(*seen.lock().expect("lock"), Some(Uuid::nil()));
        assert!(
            store
                .list_runs(RunFilter::default(), 1, 10)
                .await
                .expect("list runs")
                .items
                .is_empty(),
            "planning must not create runs"
        );
    })
    .await
    .expect("test timed out");
}
