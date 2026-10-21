//! Step name and labels of a single child started with `workflow_with`.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::time::timeout;

use ironflow_engine::config::WorkflowOptions;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::Engine;
use ironflow_engine::handler::{HandlerFuture, TypedWorkflow, WorkflowHandler};
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{RunStatus, step_trace_id};
use ironflow_store::store::Store;

use crate::fixture::{
    TEST_TIMEOUT, chain_labels, labelled, load_run, mock_provider, runs_of, start, workflow_steps,
};

/// Name of the [`Delegate`] handler.
const DELEGATE: &str = "delegate";

/// Input of [`Delegate`].
#[derive(Debug, Clone, Serialize, Deserialize)]
struct DelegateInput {
    agent: String,
}

/// A child that reports the agent it was delegated to as its output.
struct Delegate;

impl WorkflowHandler for Delegate {
    fn name(&self) -> &str {
        DELEGATE
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let input: DelegateInput = ctx.input().await?;
            ctx.set_output(&input.agent)?;
            Ok(())
        })
    }
}

impl TypedWorkflow for Delegate {
    type Input = DelegateInput;
}

/// Calls [`Delegate`] once through `workflow_with`, with the options it holds.
struct Delegator {
    options: WorkflowOptions,
}

impl WorkflowHandler for Delegator {
    fn name(&self) -> &str {
        "delegator"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let input = DelegateInput {
                agent: "alice".to_string(),
            };
            ctx.workflow_with(&Delegate, input, self.options.clone())
                .await?;
            Ok(())
        })
    }
}

fn delegator_engine(store: &Arc<InMemoryStore>, options: WorkflowOptions) -> Engine {
    let store: Arc<dyn Store> = store.clone();
    let mut engine = Engine::new(store, mock_provider());
    engine.register(Delegate).expect("register delegate");
    engine
        .register(Delegator { options })
        .expect("register delegator");
    engine
}

#[tokio::test]
async fn workflow_with_applies_step_name_and_labels_over_the_inherited_ones() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let engine = delegator_engine(
            &store,
            WorkflowOptions::new()
                .step_name("delegate-1-alice")
                .label("paperflow.io/tour", "1")
                .label("paperflow.io/agent", "alice"),
        );

        let (parent_id, result) = start(
            &engine,
            &store,
            "delegator",
            labelled(&[("team", "core"), ("paperflow.io/tour", "0")]),
        )
        .await;
        result.expect("the parent completes");

        let steps = workflow_steps(&store, parent_id).await;
        assert_eq!(steps.len(), 1);
        let step = &steps[0];
        assert_eq!(step.name, "delegate-1-alice");
        assert_eq!(
            step.trace_id,
            step_trace_id(parent_id, "delegate-1-alice", 0)
        );
        let recorded = step.input.as_ref().expect("the step records its config");
        assert_eq!(recorded["workflow_name"], DELEGATE);
        assert_eq!(
            recorded["labels"],
            json!({"paperflow.io/tour": "1", "paperflow.io/agent": "alice"})
        );

        let children = runs_of(&store, DELEGATE).await;
        assert_eq!(children.len(), 1);
        assert_eq!(
            children[0].labels,
            chain_labels(
                &[
                    ("team", "core"),
                    ("paperflow.io/tour", "1"),
                    ("paperflow.io/agent", "alice"),
                ],
                parent_id,
            )
        );
        // The step output still names the child handler, not the step.
        assert_eq!(
            step.output.as_ref().expect("output")["workflow_name"],
            DELEGATE
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn workflow_with_without_options_names_the_step_after_the_handler_and_inherits_labels() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let engine = delegator_engine(&store, WorkflowOptions::new());

        let (parent_id, result) =
            start(&engine, &store, "delegator", labelled(&[("team", "core")])).await;
        result.expect("the parent completes");

        let steps = workflow_steps(&store, parent_id).await;
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].name, DELEGATE);
        assert_eq!(steps[0].trace_id, step_trace_id(parent_id, DELEGATE, 0));
        let recorded = steps[0].input.as_ref().expect("config");
        assert!(
            recorded.get("labels").is_none(),
            "no labels recorded: {recorded}"
        );

        let children = runs_of(&store, DELEGATE).await;
        assert_eq!(children.len(), 1);
        assert_eq!(
            children[0].labels,
            chain_labels(&[("team", "core")], parent_id)
        );
        assert_eq!(
            load_run(&store, parent_id).await.status.state,
            RunStatus::Completed
        );
    })
    .await
    .expect("test timed out");
}
