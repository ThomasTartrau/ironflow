//! A child cancelled directly: its parent observes the cancellation.

use tokio::spawn;
use tokio::time::timeout;

use ironflow_engine::engine::ExecutionMode;
use ironflow_engine::error::EngineError;
use ironflow_store::models::{RunStatus, StepStatus};
use ironflow_store::store::RunStore;

use crate::fixture::{TEST_TIMEOUT, fixture};

#[tokio::test]
async fn orphan_child_cancelled_while_suspended_wakes_its_root_which_fails() {
    timeout(TEST_TIMEOUT, async {
        let fx = fixture(ExecutionMode::Local);
        let root = fx.start_suspended("asking-host").await;
        let asker = fx.wait_for_run_of("asker").await;

        let cancellation = fx.engine.cancel_run(asker.id).await.expect("cancel child");
        assert!(cancellation.cancelled_descendants.is_empty());

        fx.wait_for_status(root.id, RunStatus::Failed).await;
        let step = fx.workflow_step(root.id, "asker").await;
        assert_eq!(step.status.state, StepStatus::Failed);
        let expected = format!("child run {} was cancelled", asker.id);
        assert_eq!(step.error.as_deref(), Some(expected.as_str()));
        assert_eq!(fx.status(asker.id).await, RunStatus::Cancelled);
        // Not retried: the user stopped the child on purpose.
        assert_eq!(fx.run(root.id).await.retry_count, 0);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn orphan_child_cancelled_while_suspended_is_tolerated_with_allow_failure() {
    timeout(TEST_TIMEOUT, async {
        let fx = fixture(ExecutionMode::Local);
        let root = fx.start_suspended("tolerant-asking-host").await;
        let asker = fx.wait_for_run_of("asker").await;

        fx.engine.cancel_run(asker.id).await.expect("cancel child");

        fx.wait_for_status(root.id, RunStatus::Warning).await;
        let step = fx.workflow_step(root.id, "asker").await;
        assert_eq!(step.status.state, StepStatus::Completed);
        assert_eq!(
            *fx.reported.lock().expect("reported lock"),
            Some(RunStatus::Cancelled)
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn orphan_child_cancelled_in_a_nested_chain_fails_every_ancestor() {
    timeout(TEST_TIMEOUT, async {
        let fx = fixture(ExecutionMode::Local);
        let root = fx.start_suspended("grandparent").await;
        let host = fx.wait_for_run_of("asking-host").await;
        let asker = fx.wait_for_run_of("asker").await;

        fx.engine
            .cancel_run(asker.id)
            .await
            .expect("cancel grandchild");

        fx.wait_for_status(root.id, RunStatus::Failed).await;
        assert_eq!(fx.status(host.id).await, RunStatus::Failed);
        let host_step = fx.workflow_step(root.id, "asking-host").await;
        assert_eq!(host_step.status.state, StepStatus::Failed);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn orphan_child_cancelled_while_suspended_requeues_its_root_for_a_worker() {
    timeout(TEST_TIMEOUT, async {
        let fx = fixture(ExecutionMode::Workers);
        let root = fx.start_suspended("asking-host").await;
        let asker = fx.wait_for_run_of("asker").await;

        fx.engine.cancel_run(asker.id).await.expect("cancel child");
        assert_eq!(fx.status(root.id).await, RunStatus::Pending);

        // What a worker does once it picks the root up.
        let picked = fx
            .store
            .pick_next_pending(None)
            .await
            .expect("pick")
            .expect("the root is due");
        assert_eq!(picked.id, root.id);
        let err = fx
            .engine
            .execute_handler_run(root.id)
            .await
            .expect_err("the root fails with its child");
        assert!(
            matches!(err, EngineError::ChildRunCancelled { run_id } if run_id == asker.id),
            "got {err}"
        );
        assert_eq!(fx.status(root.id).await, RunStatus::Failed);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn orphan_child_cancelled_while_running_inline_fails_its_parent() {
    timeout(TEST_TIMEOUT, async {
        let fx = fixture(ExecutionMode::Local);
        let root = fx.picked_run("brief-host", 0).await;
        let engine = fx.engine.clone();
        let execution = spawn(async move { engine.execute_handler_run(root.id).await });

        let child = fx.wait_for_run_of("brief").await;
        fx.wait_for_step(child.id, "pause", StepStatus::Running)
            .await;
        let cancellation = fx.engine.cancel_run(child.id).await.expect("cancel child");
        assert_eq!(cancellation.run.status.state, RunStatus::Cancelled);
        // The root runs the child itself: it is not woken.
        assert_eq!(fx.status(root.id).await, RunStatus::Running);

        let err = execution
            .await
            .expect("execution task")
            .expect_err("the parent fails with its child");
        assert!(
            matches!(err, EngineError::ChildRunCancelled { run_id } if run_id == child.id),
            "got {err}"
        );
        assert_eq!(fx.status(root.id).await, RunStatus::Failed);
        assert_eq!(
            fx.status(child.id).await,
            RunStatus::Cancelled,
            "the child's own end does not overwrite its cancellation"
        );
        assert!(fx.key_is_free().await);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn orphan_child_cancelled_while_running_inline_is_tolerated_with_allow_failure() {
    timeout(TEST_TIMEOUT, async {
        let fx = fixture(ExecutionMode::Local);
        let root = fx.picked_run("tolerant-brief-host", 0).await;
        let engine = fx.engine.clone();
        let execution = spawn(async move { engine.execute_handler_run(root.id).await });

        let child = fx.wait_for_run_of("brief").await;
        fx.wait_for_step(child.id, "pause", StepStatus::Running)
            .await;
        fx.engine.cancel_run(child.id).await.expect("cancel child");

        execution
            .await
            .expect("execution task")
            .expect("the parent tolerates the cancelled child");
        assert_eq!(fx.status(root.id).await, RunStatus::Warning);
        assert_eq!(
            *fx.reported.lock().expect("reported lock"),
            Some(RunStatus::Cancelled)
        );
        assert_eq!(fx.status(child.id).await, RunStatus::Cancelled);
    })
    .await
    .expect("test timed out");
}
