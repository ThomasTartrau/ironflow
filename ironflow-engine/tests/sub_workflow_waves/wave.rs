//! A wave of children run by `workflows_with`: concurrency, step names,
//! conflicts, the shared cost cap, failures, the plan and the guard.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use rust_decimal::Decimal;
use serde_json::json;
use tokio::spawn;
use tokio::time::timeout;

use ironflow_engine::engine::EnqueueOptions;
use ironflow_engine::error::{EngineError, RUN_BUDGET_EXCEEDED_CODE};
use ironflow_engine::guard::{WorkflowGuardConfig, WorkflowRejection};
use ironflow_engine::plan::PlanOptions;
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{RunStatus, StepKind, StepStatus};

use crate::fixture::{
    Action, GatedProvider, Kit, LEAF, SPEND_USD, TEST_TIMEOUT, WAVE, WORKER, load_run,
    mock_provider, runs_of, start, task, wait_until, wave_engine, worker_run, workflow_steps,
};

fn completed(output: &str) -> String {
    format!("{}:{output}", RunStatus::Completed)
}

#[tokio::test]
async fn concurrent_children_of_a_wave_each_wait_for_the_other_to_start() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        // Run one after the other, the first child would wait forever.
        let kit = Kit::new(2);
        let entries = vec![task("alice", Action::Meet), task("bob", Action::Meet)];
        let (engine, outcomes) = wave_engine(&store, mock_provider(), &kit, entries, false);

        let (parent_id, result) = start(&engine, &store, WAVE, EnqueueOptions::default()).await;
        result.expect("the wave completes");

        // Outcomes come back in the order of the entries.
        assert_eq!(
            *outcomes.lock().expect("outcomes"),
            vec![format!("{},{}", completed("alice"), completed("bob"))]
        );
        let steps = workflow_steps(&store, parent_id).await;
        let names: Vec<&str> = steps.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["delegate-alice", "delegate-bob"]);
        assert!(steps.iter().all(|s| s.position == 0), "one wave position");
        assert!(
            steps
                .iter()
                .all(|s| s.status.state == StepStatus::Completed)
        );
        assert_eq!(runs_of(&store, WORKER).await.len(), 2);
        assert_eq!(
            load_run(&store, parent_id).await.status.state,
            RunStatus::Completed
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn duplicate_step_names_refuse_the_wave_before_anything_runs() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let kit = Kit::new(1);
        let (alice, options) = task("alice", Action::Done);
        let (bob, _) = task("bob", Action::Done);
        let entries = vec![(alice, options.clone()), (bob, options)];
        let (engine, outcomes) = wave_engine(&store, mock_provider(), &kit, entries, false);

        let (parent_id, result) = start(&engine, &store, WAVE, EnqueueOptions::default()).await;
        match result {
            Err(EngineError::StepConfig(message)) => {
                assert!(message.contains("delegate-alice"), "{message}");
            }
            other => panic!("expected a step config error, got {other:?}"),
        }

        assert!(workflow_steps(&store, parent_id).await.is_empty());
        assert!(runs_of(&store, WORKER).await.is_empty());
        assert!(outcomes.lock().expect("outcomes").is_empty());
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn an_empty_wave_records_nothing_and_the_parent_goes_on() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let kit = Kit::new(1);
        let (engine, outcomes) = wave_engine(&store, mock_provider(), &kit, Vec::new(), false);

        let (parent_id, result) = start(&engine, &store, WAVE, EnqueueOptions::default()).await;
        result.expect("the parent completes");

        assert!(workflow_steps(&store, parent_id).await.is_empty());
        assert_eq!(*outcomes.lock().expect("outcomes"), vec![String::new()]);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn a_key_held_by_an_earlier_sibling_gives_a_conflict_for_the_later_one() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let kit = Kit::new(1);
        let entries = [("alice", Action::Done), ("bob", Action::Done)]
            .into_iter()
            .map(|(agent, action)| {
                let (input, options) = task(agent, action);
                (input, options.concurrency_key("issue:7"))
            })
            .collect();
        let (engine, outcomes) = wave_engine(&store, mock_provider(), &kit, entries, false);

        let (parent_id, result) = start(&engine, &store, WAVE, EnqueueOptions::default()).await;
        result.expect("a conflict does not fail the parent");

        let alice = worker_run(&store, "alice").await;
        assert_eq!(runs_of(&store, WORKER).await.len(), 1, "no run for bob");
        assert_eq!(
            *outcomes.lock().expect("outcomes"),
            vec![format!("{},conflict:{}", completed("alice"), alice.id)]
        );
        let bob_step = workflow_steps(&store, parent_id).await.remove(1);
        assert_eq!(bob_step.name, "delegate-bob");
        assert_eq!(bob_step.status.state, StepStatus::Completed);
        assert_eq!(
            bob_step.output.expect("output")["concurrency_conflict"]["run_id"],
            json!(alice.id)
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn a_cap_with_room_for_one_child_cancels_the_other_with_run_budget_exceeded() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let kit = Kit::new(1);
        let provider = GatedProvider::new(&kit);
        // bob checks the cap while alice's agent call is still running: only
        // the budget alice holds can refuse bob, not a cost already charged.
        let entries = vec![
            task("alice", Action::Spend { after_first: false }),
            task("bob", Action::Spend { after_first: true }),
        ];
        let (engine, _) = wave_engine(&store, provider.clone(), &kit, entries, false);
        let engine = Arc::new(engine);
        let cap = Decimal::new(15, 2);
        let options = EnqueueOptions {
            max_cost_usd: Some(cap),
            ..EnqueueOptions::default()
        };

        let wave = {
            let engine = engine.clone();
            let store = store.clone();
            spawn(async move { start(&engine, &store, WAVE, options).await })
        };
        wait_until(|| async {
            runs_of(&store, WORKER)
                .await
                .iter()
                .any(|r| r.payload["agent"] == "bob" && r.status.state == RunStatus::Cancelled)
        })
        .await;
        provider.release();
        let (parent_id, result) = wave.await.expect("the wave task");

        assert!(matches!(result, Err(EngineError::RunBudgetExceeded { .. })));
        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
        let alice = worker_run(&store, "alice").await;
        assert_eq!(alice.status.state, RunStatus::Completed);
        let bob = worker_run(&store, "bob").await;
        assert_eq!(bob.status.state, RunStatus::Cancelled);
        assert!(
            bob.error
                .as_deref()
                .is_some_and(|e| e.contains(RUN_BUDGET_EXCEEDED_CODE)),
            "{:?}",
            bob.error
        );
        let parent = load_run(&store, parent_id).await;
        assert_eq!(parent.status.state, RunStatus::Cancelled);
        assert!(parent.cost_usd <= cap, "spent {}", parent.cost_usd);
        assert_eq!(
            parent.cost_usd,
            Decimal::try_from(SPEND_USD).expect("decimal")
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn a_failure_not_tolerated_is_returned_after_every_sibling_is_recorded() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let kit = Kit::new(1);
        let (alice, options) = task("alice", Action::Fail);
        let entries = vec![
            (alice, options.allow_failure()),
            task("bob", Action::Fail),
            task("carol", Action::Report),
        ];
        let (engine, outcomes) = wave_engine(&store, mock_provider(), &kit, entries, false);

        let (parent_id, result) = start(&engine, &store, WAVE, EnqueueOptions::default()).await;
        match result {
            Err(EngineError::InvalidWorkflow(message)) => assert_eq!(message, "bob refused"),
            other => panic!("expected bob's failure, got {other:?}"),
        }

        let steps = workflow_steps(&store, parent_id).await;
        assert_eq!(steps.len(), 3);
        // alice's failure is tolerated: her step completes with it.
        assert_eq!(steps[0].status.state, StepStatus::Completed);
        let tolerated = steps[0].output.as_ref().expect("alice's output");
        assert_eq!(tolerated["status"], json!(RunStatus::Failed));
        assert_eq!(steps[1].status.state, StepStatus::Failed);
        assert!(
            steps[1]
                .error
                .as_deref()
                .is_some_and(|e| e.contains("bob refused"))
        );
        // carol ran to the end although bob failed beside her.
        assert_eq!(steps[2].status.state, StepStatus::Completed);
        assert_eq!(
            worker_run(&store, "carol").await.status.state,
            RunStatus::Completed
        );
        assert_eq!(
            load_run(&store, parent_id).await.status.state,
            RunStatus::Failed
        );
        assert!(outcomes.lock().expect("outcomes").is_empty());
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn plan_records_the_wave_as_one_parallel_group_with_every_child_expanded() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let kit = Kit::new(1);
        let entries = vec![task("alice", Action::Report), task("bob", Action::Report)];
        let (engine, _) = wave_engine(&store, mock_provider(), &kit, entries, false);

        let plan = engine
            .plan_handler(WAVE, json!({}), PlanOptions::default())
            .await
            .expect("plan");

        let waves: Vec<_> = plan
            .steps
            .iter()
            .filter(|s| s.kind == StepKind::Workflow)
            .collect();
        assert_eq!(waves.len(), 2);
        assert_eq!(waves[0].name, "delegate-alice");
        assert_eq!(waves[1].name, "delegate-bob");
        assert!(waves[0].parallel_group.is_some());
        assert_eq!(waves[0].parallel_group, waves[1].parallel_group);
        assert!(
            waves[1].depends_on.is_empty(),
            "siblings do not wait on each other"
        );

        let reports: Vec<_> = plan.steps.iter().filter(|s| s.name == "report").collect();
        assert_eq!(reports.len(), 2, "every child is expanded");
        assert_eq!(reports[0].depends_on, vec!["delegate-alice".to_string()]);
        assert_eq!(reports[1].depends_on, vec!["delegate-bob".to_string()]);
        assert!(reports.iter().all(|s| s.depth == 1 && s.workflow == WORKER));
        // No run is created while planning.
        assert!(runs_of(&store, WORKER).await.is_empty());
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn the_guard_counts_each_child_and_refuses_a_wave_past_max_fan_out() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let kit = Kit::new(1);
        let entries = vec![task("alice", Action::Done), task("bob", Action::Done)];
        let (engine, _) = wave_engine(&store, mock_provider(), &kit, entries, false);
        let engine = engine.with_guard_config(WorkflowGuardConfig::new().with_max_fan_out(1));

        let (parent_id, result) = start(&engine, &store, WAVE, EnqueueOptions::default()).await;
        match result {
            Err(EngineError::WorkflowGuardRejected(WorkflowRejection::MaxFanOutExceeded {
                invocations,
                max,
            })) => assert_eq!((invocations, max), (2, 1)),
            other => panic!("expected a fan-out rejection, got {other:?}"),
        }
        assert!(workflow_steps(&store, parent_id).await.is_empty());
        assert!(runs_of(&store, WORKER).await.is_empty());
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn the_guard_gives_each_sibling_its_own_depth_and_chain_and_shares_the_totals() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let kit = Kit::new(1);
        let entries = vec![
            task("alice", Action::Nest),
            task("bob", Action::Nest),
            task("carol", Action::Nest),
        ];
        let (engine, _) = wave_engine(&store, mock_provider(), &kit, entries, false);
        // Each worker runs at depth 1, so its leaf fits a depth of 2 only if
        // the siblings do not stack on one another. Three workers and three
        // leaves fit a fan-out of six exactly: the count is shared.
        let config = WorkflowGuardConfig::new()
            .with_max_depth(2)
            .with_max_fan_out(6);
        let engine = engine.with_guard_config(config);

        let (_, result) = start(&engine, &store, WAVE, EnqueueOptions::default()).await;
        result.expect("every sibling fits on its own branch");
        assert_eq!(runs_of(&store, WORKER).await.len(), 3);
        assert_eq!(runs_of(&store, LEAF).await.len(), 3);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn the_guard_counts_the_children_of_siblings_against_one_fan_out() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let kit = Kit::new(1);
        let entries = vec![task("alice", Action::Nest), task("bob", Action::Nest)];
        let (engine, _) = wave_engine(&store, mock_provider(), &kit, entries, false);
        // Two workers fit; their two leaves would make four.
        let engine = engine.with_guard_config(WorkflowGuardConfig::new().with_max_fan_out(3));

        // Whichever sibling claims the last slot first, the other is refused.
        let (parent_id, result) = start(&engine, &store, WAVE, EnqueueOptions::default()).await;
        match result {
            Err(EngineError::WorkflowGuardRejected(WorkflowRejection::MaxFanOutExceeded {
                invocations,
                max,
            })) => assert_eq!((invocations, max), (3, 3)),
            other => panic!("expected the second leaf to be refused, got {other:?}"),
        }
        assert_eq!(runs_of(&store, LEAF).await.len(), 1);
        // A guard stop cancels the run, it does not fail it.
        assert_eq!(
            load_run(&store, parent_id).await.status.state,
            RunStatus::Cancelled
        );
    })
    .await
    .expect("test timed out");
}
