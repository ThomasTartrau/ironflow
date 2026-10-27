//! What becomes of the children of a round: a failed child, a role answer
//! that is not its output type, and a round whose tasks are all refused.

use serde_json::json;
use tokio::time::timeout;

use ironflow_store::models::{RunStatus, StepStatus};

use crate::fixture::{
    FAILING_TASK, LeadWorkflow, MALFORMED_TASK, Script, TEST_TIMEOUT, answer, child_runs, harness,
};

#[tokio::test]
async fn children_failed_child_fails_the_supervisor_run() {
    timeout(TEST_TIMEOUT, async {
        let script = Script::new([
            json!({"delegate": [{"Coder": FAILING_TASK}, {"Reviewer": "the theme"}]}),
            answer("unreachable"),
        ]);
        let mut engine = harness(LeadWorkflow::new(), &script);
        let result = engine.run(json!({})).await.expect("run");

        assert_eq!(result.status(), RunStatus::Failed);
        let error = result.error().expect("an error");
        assert!(error.contains("the coder crashed"), "got: {error}");
        assert_eq!(
            script.calls_of("Lead").len(),
            1,
            "no round after a failed child"
        );
        // The sibling still ran to the end before the failure was returned.
        let children = child_runs(engine.store().as_ref(), result.run_id()).await;
        let mut statuses: Vec<RunStatus> = children.iter().map(|run| run.status.state).collect();
        statuses.sort_by_key(|status| status.to_string());
        assert_eq!(statuses, vec![RunStatus::Completed, RunStatus::Failed]);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn children_answer_that_is_not_the_role_output_fails_the_child() {
    timeout(TEST_TIMEOUT, async {
        let script = Script::new([
            json!({"delegate": [{"Reviewer": MALFORMED_TASK}]}),
            answer("unreachable"),
        ]);
        let mut engine = harness(LeadWorkflow::new(), &script);
        let result = engine.run(json!({})).await.expect("run");

        assert_eq!(result.status(), RunStatus::Failed);
        let children = child_runs(engine.store().as_ref(), result.run_id()).await;
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].status.state, RunStatus::Failed);
        let error = children[0].error.as_deref().expect("a child error");
        assert!(error.starts_with("serialization error"), "got: {error}");
        assert_eq!(script.calls_of("Lead").len(), 1);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn children_round_with_every_task_refused_goes_on_without_children() {
    timeout(TEST_TIMEOUT, async {
        let script = Script::new([
            json!({"delegate": [{"Hacker": "a"}, "Intern"]}),
            answer("done alone"),
        ]);
        let mut engine = harness(LeadWorkflow::new(), &script);
        let result = engine.run(json!({})).await.expect("run");

        assert_eq!(
            result.status(),
            RunStatus::Completed,
            "{:?}",
            result.error()
        );
        assert_eq!(result.output(), &json!({"summary": "done alone"}));
        assert_eq!(
            result.step_names(),
            vec![
                "lead/turn-1",
                "lead/turn-1/1-refused",
                "lead/turn-1/2-refused",
                "lead/turn-2"
            ]
        );
        assert_eq!(
            result.step("lead/turn-1/2-refused").status(),
            StepStatus::Skipped
        );
        assert!(
            child_runs(engine.store().as_ref(), result.run_id())
                .await
                .is_empty()
        );
        let second = &script.calls_of("Lead")[1].prompt;
        assert_eq!(second.matches("was refused").count(), 2);
    })
    .await
    .expect("test timed out");
}
