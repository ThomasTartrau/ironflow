//! A supervisor resumes where it stopped: after an approval in a child run,
//! after a lost lease, and in plan mode it stops after its first turn.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::json;
use tokio::spawn;
use tokio::time::{sleep, timeout};

use ironflow_core::provider::AgentProvider;
use ironflow_engine::config::DEFAULT_RESUME_PROMPT;
use ironflow_engine::engine::Engine;
use ironflow_engine::handler::TeamWorkflow;
use ironflow_engine::plan::PlanOptions;
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{LeaseRequest, RunStatus, TriggerKind};
use ironflow_store::store::{RunStore, STEP_INTERRUPTED_ERROR, Store};

use crate::fixture::{
    LeadWorkflow, Script, Squad, TEST_TIMEOUT, WORKFLOW, answer, child_runs, harness,
};

/// An engine running [`LeadWorkflow`] and its team on `script` over `store`.
fn engine_on(store: &Arc<InMemoryStore>, script: &Arc<Script>) -> Engine {
    let provider: Arc<dyn AgentProvider> = script.clone();
    let dyn_store: Arc<dyn Store> = store.clone();
    let mut engine = Engine::new(dyn_store, provider);
    engine.register(LeadWorkflow::new()).expect("register");
    engine
        .register(TeamWorkflow::<Squad>::new())
        .expect("register");
    engine
}

/// Pick the run the way a worker does, with a lease that expires at once.
async fn pick(store: &InMemoryStore) {
    store
        .pick_next_pending(Some(LeaseRequest {
            worker_id: "worker-that-dies".to_string(),
            ttl: Duration::from_millis(1),
        }))
        .await
        .expect("pick")
        .expect("a pending run");
}

#[tokio::test]
async fn resume_after_an_approval_in_a_child_reruns_nothing() {
    timeout(TEST_TIMEOUT, async {
        let script = Script::new([
            json!({"delegate": [{"Reviewer": "the theme"}, {"Deployer": "v2"}]}),
            answer("deployed"),
        ]);
        let mut engine = harness(LeadWorkflow::new(), &script);
        let suspended = engine.run(json!({})).await.expect("run");
        assert_eq!(suspended.status(), RunStatus::AwaitingApproval);
        assert!(
            script.calls_of("Deployer").is_empty(),
            "the gate holds the agent"
        );

        let store = engine.store();
        let children = child_runs(store.as_ref(), suspended.run_id()).await;
        let gated = children
            .iter()
            .find(|run| run.status.state == RunStatus::AwaitingApproval)
            .expect("the deployer run waits for its approval");

        let result = engine.resume(gated.id).await.expect("resume");
        assert_eq!(
            result.status(),
            RunStatus::Completed,
            "{:?}",
            result.error()
        );
        assert_eq!(result.output(), &json!({"summary": "deployed"}));
        assert_eq!(script.calls_of("Lead").len(), 2, "turn 1 is not run again");
        assert_eq!(
            script.calls_of("Reviewer").len(),
            1,
            "a finished child is not run again"
        );
        assert_eq!(script.calls_of("Deployer").len(), 1);
        assert_eq!(
            child_runs(store.as_ref(), suspended.run_id()).await.len(),
            2
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn resume_after_a_lost_lease_reruns_only_the_interrupted_turn() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let script = Arc::new(Script {
            lead_turns: Mutex::new(
                [
                    json!({"delegate": [{"Coder": "the toggle"}]}),
                    answer("shipped"),
                ]
                .into_iter()
                .collect(),
            ),
            hang_on_lead_call: Some(2),
            ..Script::default()
        });
        let engine = Arc::new(engine_on(&store, &script));

        let run_id = engine
            .enqueue_handler(WORKFLOW, TriggerKind::Manual, json!({}), 3)
            .await
            .expect("enqueue")
            .id;
        pick(&store).await;

        // Kill the worker while the supervisor runs its second turn.
        let worker = engine.clone();
        let task = spawn(async move { worker.execute_handler_run(run_id).await });
        while !script.hung.load(Ordering::SeqCst) {
            sleep(Duration::from_millis(5)).await;
        }
        task.abort();
        assert!(
            task.await
                .expect_err("the worker was killed")
                .is_cancelled()
        );

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

        pick(&store).await;
        engine
            .execute_handler_run(run_id)
            .await
            .expect("the requeued run completes");
        let run = store.get_run(run_id).await.expect("get").expect("run");
        assert_eq!(run.status.state, RunStatus::Completed, "{:?}", run.error);

        // Turn 1 and the coder ran once; turn 2 was interrupted, then ran again.
        let lead = script.calls_of("Lead");
        assert_eq!(lead.len(), 3);
        assert_eq!(script.calls_of("Coder").len(), 1);
        // The interrupted turn holds the coder result and resumes its own
        // session, like any interrupted agent step.
        assert!(lead[1].prompt.contains("Implement: the toggle"));
        assert!(lead[1].session_id.is_some());
        assert_eq!(lead[2].resume_session_id, lead[1].session_id);
        assert_eq!(lead[2].prompt, DEFAULT_RESUME_PROMPT);
        assert_eq!(child_runs(store.as_ref(), run_id).await.len(), 1);
        let turns: Vec<_> = store
            .list_steps(run_id)
            .await
            .expect("steps")
            .into_iter()
            .filter(|step| step.name == "lead/turn-2")
            .collect();
        assert_eq!(turns.len(), 2);
        assert!(
            turns
                .iter()
                .any(|step| step.error.as_deref() == Some(STEP_INTERRUPTED_ERROR))
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn resume_plan_stops_after_the_first_turn() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let script = Script::new([]);
        let engine = engine_on(&store, &script);

        let plan = engine
            .plan_handler(WORKFLOW, json!({}), PlanOptions::default())
            .await
            .expect("plan");

        let names: Vec<&str> = plan.steps.iter().map(|step| step.name.as_str()).collect();
        assert_eq!(names, vec!["lead/turn-1"]);
        assert!(plan.truncated);
        let reason = plan.incomplete_reason.expect("a reason");
        assert!(reason.contains("the plan stops here"), "got: {reason}");
        assert!(
            script.calls.lock().expect("lock").is_empty(),
            "no agent is called"
        );
    })
    .await
    .expect("test timed out");
}
