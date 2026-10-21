//! A wave across executions: suspension and resume, replay, lost lease, and
//! several suspended children resumed while their root runs.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use serde_json::json;
use tokio::time::{sleep, timeout};
use tokio::{join, spawn};
use uuid::Uuid;

use ironflow_engine::engine::{Engine, EnqueueOptions};
use ironflow_engine::notify::Event;
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{RunStatus, StepStatus, TriggerKind};
use ironflow_store::store::{RunStore, STEP_INTERRUPTED_ERROR};

use crate::fixture::{
    Action, EventCollector, Kit, TEST_TIMEOUT, WAVE, WORKER, answer, load_run, mock_provider,
    pick_with_expiring_lease, runs_of, start, task, wait_until, wave_engine, worker_run,
    workflow_steps,
};

fn completed(output: &str) -> String {
    format!("{}:{output}", RunStatus::Completed)
}

#[tokio::test]
async fn a_suspended_child_of_a_wave_resumes_without_creating_a_child_twice() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let kit = Kit::new(1);
        let entries = vec![task("alice", Action::Done), task("bob", Action::Ask)];
        let (engine, outcomes) = wave_engine(&store, mock_provider(), &kit, entries, false);

        let (parent_id, result) = start(&engine, &store, WAVE, EnqueueOptions::default()).await;
        result.expect("the wave suspends");
        assert_eq!(
            load_run(&store, parent_id).await.status.state,
            RunStatus::AwaitingApproval
        );
        let alice = worker_run(&store, "alice").await;
        assert_eq!(alice.status.state, RunStatus::Completed);
        let bob = worker_run(&store, "bob").await;
        assert_eq!(bob.status.state, RunStatus::AwaitingApproval);
        let steps = workflow_steps(&store, parent_id).await;
        assert_eq!(steps[0].status.state, StepStatus::Completed);
        assert_eq!(
            steps[1].status.state,
            StepStatus::Running,
            "bob's step stays open"
        );
        assert!(outcomes.lock().expect("outcomes").is_empty());

        answer(&store, bob.id, "Ada").await;
        let resumed = engine.resume_run(bob.id).await.expect("the chain resumes");

        assert_eq!(resumed.run.id, parent_id);
        assert_eq!(resumed.run.status.state, RunStatus::Completed);
        assert_eq!(
            *outcomes.lock().expect("outcomes"),
            vec![format!("{},{}", completed("alice"), completed("Ada"))]
        );
        let runs = runs_of(&store, WORKER).await;
        assert_eq!(runs.len(), 2, "no child is created twice");
        assert_eq!(worker_run(&store, "alice").await.id, alice.id);
        assert_eq!(workflow_steps(&store, parent_id).await.len(), 2);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn a_completed_wave_is_replayed_without_a_new_child() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let kit = Kit::new(1);
        let mut entries: Vec<_> = [("alice", Action::Done), ("bob", Action::Done)]
            .into_iter()
            .map(|(agent, action)| {
                let (input, options) = task(agent, action);
                (input, options.concurrency_key("issue:9"))
            })
            .collect();
        entries.push(task("carol", Action::Done));
        // The parent waits for a human after the wave, so it replays the wave
        // once the human answers.
        let (engine, outcomes) = wave_engine(&store, mock_provider(), &kit, entries, true);

        let (parent_id, result) = start(&engine, &store, WAVE, EnqueueOptions::default()).await;
        result.expect("the parent suspends after the wave");
        let first = outcomes.lock().expect("outcomes")[0].clone();
        let alice = worker_run(&store, "alice").await;
        assert_eq!(
            first,
            format!(
                "{},conflict:{},{}",
                completed("alice"),
                alice.id,
                completed("carol")
            )
        );

        answer(&store, parent_id, "go").await;
        let resumed = engine.resume_run(parent_id).await.expect("resume");
        assert_eq!(resumed.run.status.state, RunStatus::Completed);

        // The replayed wave hands the handler the same outcomes, the conflict
        // included, and creates nothing.
        assert_eq!(
            *outcomes.lock().expect("outcomes"),
            vec![first.clone(), first]
        );
        assert_eq!(runs_of(&store, WORKER).await.len(), 2);
        assert_eq!(workflow_steps(&store, parent_id).await.len(), 3);
    })
    .await
    .expect("test timed out");
}

/// Execute `run_id` on a task and kill it once `kit` saw `holds` hold points.
async fn crash_inside(engine: &Arc<Engine>, run_id: Uuid, kit: &Kit, holds: u32) {
    let worker = engine.clone();
    let task = spawn(async move { worker.execute_handler_run(run_id).await });
    while kit.holds.load(Ordering::SeqCst) < holds {
        sleep(Duration::from_millis(5)).await;
    }
    task.abort();
    let err = task.await.expect_err("the worker was killed");
    assert!(err.is_cancelled());
}

#[tokio::test]
async fn lease_loss_reenters_the_child_runs_of_the_interrupted_wave_steps() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let kit = Kit::new(1);
        kit.hold.store(true, Ordering::SeqCst);
        let entries = vec![task("alice", Action::Hold), task("bob", Action::Hold)];
        let (engine, _) = wave_engine(&store, mock_provider(), &kit, entries, false);
        let engine = Arc::new(engine);

        let run_id = engine
            .enqueue_handler_with_options(
                WAVE,
                TriggerKind::Manual,
                json!({}),
                EnqueueOptions {
                    max_retries: 3,
                    ..EnqueueOptions::default()
                },
            )
            .await
            .expect("enqueue")
            .into_run()
            .id;
        pick_with_expiring_lease(&store, run_id).await;
        // Both children hold when the worker dies.
        crash_inside(&engine, run_id, &kit, 2).await;

        sleep(Duration::from_millis(5)).await;
        let reaped = store.reap_expired_leases(100).await.expect("reap");
        assert!(
            reaped
                .iter()
                .any(|r| r.run.id == run_id && r.to == RunStatus::Pending)
        );
        engine
            .interrupt_running_steps(run_id)
            .await
            .expect("interrupt running steps");

        kit.release_all();
        pick_with_expiring_lease(&store, run_id).await;
        engine
            .execute_handler_run(run_id)
            .await
            .expect("the requeued run completes");

        let runs = runs_of(&store, WORKER).await;
        assert_eq!(runs.len(), 2, "one child run per entry");
        assert!(runs.iter().all(|r| r.status.state == RunStatus::Completed));
        assert_eq!(kit.holds.load(Ordering::SeqCst), 4);

        let steps = workflow_steps(&store, run_id).await;
        assert_eq!(steps.len(), 4, "{steps:?}");
        for name in ["delegate-alice", "delegate-bob"] {
            let named: Vec<_> = steps.iter().filter(|s| s.name == name).collect();
            assert_eq!(named.len(), 2);
            assert_eq!(named[0].status.state, StepStatus::Failed);
            assert_eq!(named[0].error.as_deref(), Some(STEP_INTERRUPTED_ERROR));
            assert_eq!(named[1].status.state, StepStatus::Completed);
            let interrupted_child = &named[0].output.as_ref().expect("child id")["child_run_id"];
            let finished_child = &named[1].output.as_ref().expect("output")["run_id"];
            assert_eq!(
                interrupted_child, finished_child,
                "{name} re-enters its child"
            );
        }
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn a_sibling_resolved_while_the_root_runs_waits_for_it_then_resumes_it() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let kit = Kit::new(1);
        kit.hold.store(true, Ordering::SeqCst);
        let entries = vec![task("alice", Action::AskThenHold), task("bob", Action::Ask)];
        let (engine, outcomes) = wave_engine(&store, mock_provider(), &kit, entries, false);
        let engine = Arc::new(engine);

        let (parent_id, result) = start(&engine, &store, WAVE, EnqueueOptions::default()).await;
        result.expect("the wave suspends");
        let alice = worker_run(&store, "alice").await;
        let bob = worker_run(&store, "bob").await;

        // alice's answer resumes the root, which holds inside alice.
        answer(&store, alice.id, "Ada").await;
        let first = {
            let engine = engine.clone();
            spawn(async move { engine.resume_run(alice.id).await })
        };
        // The root re-entered bob, still unanswered, and suspended him again.
        wait_until(|| async {
            kit.asks.load(Ordering::SeqCst) == 4
                && kit.holds.load(Ordering::SeqCst) == 1
                && load_run(&store, bob.id).await.status.state == RunStatus::AwaitingApproval
        })
        .await;

        // bob's answer arrives while the root runs.
        answer(&store, bob.id, "Bea").await;
        let second = {
            let engine = engine.clone();
            spawn(async move { engine.resume_run(bob.id).await })
        };
        // Let the second resume find the root running before alice lets go.
        sleep(Duration::from_millis(200)).await;
        assert_ne!(
            load_run(&store, bob.id).await.status.state,
            RunStatus::Failed,
            "a busy root must not fail the child"
        );
        kit.release_all();

        let first = first.await.expect("first task").expect("first resume");
        assert_eq!(first.run.id, parent_id);
        let second = second.await.expect("second task").expect("second resume");
        assert_eq!(second.run.id, parent_id);

        assert_eq!(
            load_run(&store, parent_id).await.status.state,
            RunStatus::Completed
        );
        assert_eq!(
            *outcomes.lock().expect("outcomes"),
            vec![format!("{},{}", completed("Ada"), completed("Bea"))]
        );
        assert_eq!(runs_of(&store, WORKER).await.len(), 2);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn siblings_resumed_together_start_the_root_once() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let kit = Kit::new(1);
        let entries = vec![task("alice", Action::Ask), task("bob", Action::Ask)];
        let (engine, outcomes) = wave_engine(&store, mock_provider(), &kit, entries, false);
        let engine = Arc::new(engine);

        let (parent_id, result) = start(&engine, &store, WAVE, EnqueueOptions::default()).await;
        result.expect("the wave suspends");
        let alice = worker_run(&store, "alice").await;
        let bob = worker_run(&store, "bob").await;

        // Both answers land before either resume, like two capacity waits
        // woken by one waker tick.
        answer(&store, alice.id, "Ada").await;
        answer(&store, bob.id, "Bea").await;
        let (one, two) = join!(
            spawn({
                let engine = engine.clone();
                async move { engine.resume_run(alice.id).await }
            }),
            spawn({
                let engine = engine.clone();
                async move { engine.resume_run(bob.id).await }
            }),
        );
        one.expect("first task").expect("first resume");
        two.expect("second task").expect("second resume");

        assert_eq!(
            load_run(&store, parent_id).await.status.state,
            RunStatus::Completed
        );
        // The root ran past the wave exactly once.
        assert_eq!(
            *outcomes.lock().expect("outcomes"),
            vec![format!("{},{}", completed("Ada"), completed("Bea"))]
        );
        let runs = runs_of(&store, WORKER).await;
        assert_eq!(runs.len(), 2);
        assert!(runs.iter().all(|r| r.status.state == RunStatus::Completed));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn every_suspended_child_of_a_wave_publishes_its_approval_event() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let kit = Kit::new(1);
        let entries = vec![task("alice", Action::Approve), task("bob", Action::Approve)];
        let (mut engine, _) = wave_engine(&store, mock_provider(), &kit, entries, false);
        let collector = EventCollector::default();
        engine.subscribe(collector.clone(), Event::ALL);

        let (parent_id, result) = start(&engine, &store, WAVE, EnqueueOptions::default()).await;
        result.expect("the wave suspends");
        assert_eq!(
            load_run(&store, parent_id).await.status.state,
            RunStatus::AwaitingApproval
        );

        let expected: HashSet<Uuid> = [
            worker_run(&store, "alice").await.id,
            worker_run(&store, "bob").await.id,
        ]
        .into_iter()
        .collect();
        // Subscribers run in spawned tasks.
        wait_until(|| async {
            let requested: HashSet<Uuid> = collector
                .events
                .lock()
                .expect("events")
                .iter()
                .filter_map(|event| match event {
                    Event::ApprovalRequested(requested) => Some(requested.run_id),
                    _ => None,
                })
                .collect();
            requested == expected
        })
        .await;
    })
    .await
    .expect("test timed out");
}
