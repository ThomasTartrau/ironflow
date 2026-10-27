//! What bounds a supervisor: its limits, the workflow guard, the cost cap of
//! the run tree, malformed turns and a team that is not registered.

use rust_decimal::Decimal;
use serde_json::json;
use tokio::time::timeout;

use ironflow_engine::config::{MAX_SUPERVISOR_NAME_LEN, SupervisionLimits};
use ironflow_engine::guard::WorkflowGuardConfig;
use ironflow_engine::testing::TestEngine;
use ironflow_store::models::RunStatus;

use crate::fixture::{LeadWorkflow, Script, TEST_TIMEOUT, answer, child_runs, harness};

#[tokio::test]
async fn limits_round_over_the_task_limit_fails_before_any_child() {
    timeout(TEST_TIMEOUT, async {
        let script = Script::new([json!({"delegate": [{"Coder": "a"}, {"Coder": "b"}]})]);
        let handler =
            LeadWorkflow::new().limits(SupervisionLimits::default().max_tasks_per_round(1));
        let mut engine = harness(handler, &script);
        let result = engine.run(json!({})).await.expect("run");

        assert_eq!(result.status(), RunStatus::Failed);
        let error = result.error().expect("an error");
        assert!(
            error.contains("delegates 2 tasks, more than the 1 allowed per round"),
            "got: {error}"
        );
        assert!(
            child_runs(engine.store().as_ref(), result.run_id())
                .await
                .is_empty()
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn limits_round_at_the_task_limit_runs() {
    timeout(TEST_TIMEOUT, async {
        let script = Script::new([
            json!({"delegate": [{"Reviewer": "a"}, {"Reviewer": "b"}]}),
            answer("reviewed"),
        ]);
        let handler =
            LeadWorkflow::new().limits(SupervisionLimits::default().max_tasks_per_round(2));
        let mut engine = harness(handler, &script);
        let result = engine.run(json!({})).await.expect("run");

        assert_eq!(
            result.status(),
            RunStatus::Completed,
            "{:?}",
            result.error()
        );
        assert_eq!(
            child_runs(engine.store().as_ref(), result.run_id())
                .await
                .len(),
            2
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
#[should_panic(expected = "supervisor name must be at most 100 bytes")]
async fn limits_supervisor_name_over_the_limit_panics() {
    // Prefixed to every step name, it must leave room for the role name in
    // the 255 characters of a stored step name.
    let script = Script::new([answer("unreachable")]);
    let mut handler = LeadWorkflow::new();
    handler.supervisor = "l".repeat(MAX_SUPERVISOR_NAME_LEN + 1);
    let mut engine = harness(handler, &script);
    let _ = engine.run(json!({})).await;
}

#[tokio::test]
async fn limits_turn_without_tasks_or_answer_fails() {
    timeout(TEST_TIMEOUT, async {
        let script = Script::new([json!({"delegate": []})]);
        let mut engine = harness(LeadWorkflow::new(), &script);
        let result = engine.run(json!({})).await.expect("run");

        assert_eq!(result.status(), RunStatus::Failed);
        let error = result.error().expect("an error");
        assert!(
            error.contains("turn 1 neither delegates nor answers"),
            "got: {error}"
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn limits_turn_with_tasks_and_answer_fails() {
    timeout(TEST_TIMEOUT, async {
        let script = Script::new([json!({
            "delegate": [{"Coder": "a"}],
            "answer": {"summary": "both"}
        })]);
        let mut engine = harness(LeadWorkflow::new(), &script);
        let result = engine.run(json!({})).await.expect("run");

        assert_eq!(result.status(), RunStatus::Failed);
        let error = result.error().expect("an error");
        assert!(
            error.contains("turn 1 both delegates and answers"),
            "got: {error}"
        );
        assert!(
            child_runs(engine.store().as_ref(), result.run_id())
                .await
                .is_empty()
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn limits_turn_that_is_not_a_turn_fails() {
    timeout(TEST_TIMEOUT, async {
        let script = Script::new([json!({"delegate": "everything"})]);
        let mut engine = harness(LeadWorkflow::new(), &script);
        let result = engine.run(json!({})).await.expect("run");

        assert_eq!(result.status(), RunStatus::Failed);
        let error = result.error().expect("an error");
        assert!(error.starts_with("serialization error"), "got: {error}");
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn limits_still_delegating_after_the_last_round_fails() {
    timeout(TEST_TIMEOUT, async {
        let script = Script::new([
            json!({"delegate": [{"Reviewer": "a"}]}),
            json!({"delegate": [{"Reviewer": "b"}]}),
        ]);
        let handler = LeadWorkflow::new().limits(SupervisionLimits::default().max_rounds(2));
        let mut engine = harness(handler, &script);
        let result = engine.run(json!({})).await.expect("run");

        assert_eq!(result.status(), RunStatus::Failed);
        let error = result.error().expect("an error");
        assert!(
            error.contains("supervisor \"lead\" failed: still delegating after 2 rounds"),
            "got: {error}"
        );
        assert_eq!(
            child_runs(engine.store().as_ref(), result.run_id())
                .await
                .len(),
            2
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn limits_wave_over_the_guard_fan_out_starts_no_child() {
    timeout(TEST_TIMEOUT, async {
        let script = Script::new([json!({"delegate": [{"Coder": "a"}, {"Coder": "b"}]})]);
        let mut handler = LeadWorkflow::new();
        handler.guard = Some(WorkflowGuardConfig::new().with_max_fan_out(1));
        let mut engine = harness(handler, &script);
        let result = engine.run(json!({})).await.expect("run");

        assert_eq!(result.status(), RunStatus::Cancelled);
        let error = result.error().expect("an error");
        assert!(error.contains("WORKFLOW_GUARD_REJECTED"), "got: {error}");
        assert!(
            child_runs(engine.store().as_ref(), result.run_id())
                .await
                .is_empty()
        );
        assert!(script.calls_of("Coder").is_empty());
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn limits_children_share_the_cost_cap_of_the_tree() {
    timeout(TEST_TIMEOUT, async {
        // The coder holds 0.10 USD while it runs: over a cap of 0.05.
        let script = Script::new([json!({"delegate": [{"Coder": "a"}]})]);
        let mut handler = LeadWorkflow::new();
        handler.max_cost = Some(Decimal::new(5, 2));
        let mut engine = harness(handler, &script);
        let result = engine.run(json!({})).await.expect("run");

        assert_eq!(result.status(), RunStatus::Cancelled);
        let children = child_runs(engine.store().as_ref(), result.run_id()).await;
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].status.state, RunStatus::Cancelled);
        assert!(
            children[0]
                .error
                .as_deref()
                .is_some_and(|error| error.contains("RUN_BUDGET_EXCEEDED")),
            "got: {:?}",
            children[0].error
        );
        assert!(
            script.calls_of("Coder").is_empty(),
            "the cap refused the step"
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn limits_team_not_registered_fails_at_the_first_delegation() {
    timeout(TEST_TIMEOUT, async {
        let script = Script::new([json!({"delegate": [{"Coder": "a"}]})]);
        let mut engine = TestEngine::new()
            .with_handler(LeadWorkflow::new())
            .with_agent_provider(script.clone());
        let result = engine.run(json!({})).await.expect("run");

        assert_eq!(result.status(), RunStatus::Failed);
        let error = result.error().expect("an error");
        assert!(
            error.contains("no handler registered: squad"),
            "got: {error}"
        );
    })
    .await
    .expect("test timed out");
}
