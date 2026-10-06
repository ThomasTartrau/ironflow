//! A parent that stops (fails, retries, is cancelled) leaves no child active.

use std::time::Duration;

use tokio::time::{sleep, timeout};
use uuid::Uuid;

use ironflow_engine::engine::ExecutionMode;
use ironflow_engine::error::EngineError;
use ironflow_store::error::StoreError;
use ironflow_store::models::{RunStatus, TriggerKind};
use ironflow_store::store::RunStore;
use serde_json::json;

use crate::fixture::{TEST_TIMEOUT, fixture};

#[tokio::test]
async fn orphan_child_is_cancelled_when_its_root_fails_for_good() {
    timeout(TEST_TIMEOUT, async {
        let fx = fixture(ExecutionMode::Local);
        let (root, child) = fx.abandon_with_stuck_child(0).await;

        let status = fx
            .engine
            .fail_or_schedule_retry(root.id, "run timed out after 1s", true, None, None)
            .await
            .expect("record the timeout");
        assert_eq!(status, RunStatus::Failed);

        let child = fx.run(child.id).await;
        assert_eq!(child.status.state, RunStatus::Cancelled);
        assert!(
            child
                .error
                .as_deref()
                .is_some_and(|e| e.contains("run timed out after 1s")),
            "got {:?}",
            child.error
        );
        assert!(fx.open_steps(child.id).await.is_empty());
        fx.events.wait_for_cancelled(child.id).await;
        assert!(fx.key_is_free().await);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn orphan_child_is_cancelled_when_its_root_schedules_a_retry() {
    timeout(TEST_TIMEOUT, async {
        let fx = fixture(ExecutionMode::Local);
        let (root, child) = fx.abandon_with_stuck_child(2).await;

        let status = fx
            .engine
            .fail_or_schedule_retry(root.id, "parent run panicked", true, None, None)
            .await
            .expect("record the panic");
        assert_eq!(status, RunStatus::Retrying);

        // The next attempt starts its own child: the key must be free for it.
        assert_eq!(fx.status(child.id).await, RunStatus::Cancelled);
        assert!(fx.key_is_free().await);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn orphan_child_of_a_cancelled_root_is_cancelled_down_the_chain() {
    timeout(TEST_TIMEOUT, async {
        let fx = fixture(ExecutionMode::Local);
        let root = fx.start_suspended("grandparent").await;
        let host = fx.wait_for_run_of("asking-host").await;
        let asker = fx.wait_for_run_of("asker").await;

        let cancellation = fx.engine.cancel_run(root.id).await.expect("cancel root");

        assert_eq!(cancellation.run.status.state, RunStatus::Cancelled);
        assert_eq!(cancellation.cancelled_descendants, [host.id, asker.id]);
        for run_id in [root.id, host.id, asker.id] {
            assert_eq!(fx.status(run_id).await, RunStatus::Cancelled);
            assert!(fx.open_steps(run_id).await.is_empty());
            fx.events.wait_for_cancelled(run_id).await;
        }
        assert!(fx.key_is_free().await);
        assert!(
            fx.store
                .list_active_descendants(root.id)
                .await
                .unwrap()
                .is_empty()
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn orphan_child_cancel_is_idempotent() {
    timeout(TEST_TIMEOUT, async {
        let fx = fixture(ExecutionMode::Local);
        let root = fx.start_suspended("asking-host").await;
        let first = fx.engine.cancel_run(root.id).await.expect("first cancel");
        assert_eq!(first.cancelled_descendants.len(), 1);
        fx.events.wait_for_cancelled(root.id).await;

        let second = fx.engine.cancel_run(root.id).await.expect("second cancel");
        assert!(second.cancelled_descendants.is_empty());
        assert_eq!(second.run.status.state, RunStatus::Cancelled);
        sleep(Duration::from_millis(100)).await;
        let root_events = fx
            .events
            .cancelled()
            .into_iter()
            .filter(|id| *id == root.id)
            .count();
        assert_eq!(root_events, 1, "a second cancel publishes nothing");
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn orphan_child_cancel_refuses_a_finished_or_unknown_run() {
    timeout(TEST_TIMEOUT, async {
        let fx = fixture(ExecutionMode::Local);
        let done = fx
            .engine
            .run_handler("tolerant-brief-host", TriggerKind::Manual, json!({}))
            .await
            .expect("run completes")
            .run;
        assert_eq!(done.status.state, RunStatus::Completed);

        let err = fx.engine.cancel_run(done.id).await.expect_err("finished");
        assert!(
            matches!(
                err,
                EngineError::Store(StoreError::InvalidTransition {
                    from: RunStatus::Completed,
                    to: RunStatus::Cancelled
                })
            ),
            "got {err}"
        );

        let err = fx
            .engine
            .cancel_run(Uuid::now_v7())
            .await
            .expect_err("unknown");
        assert!(
            matches!(err, EngineError::Store(StoreError::RunNotFound(_))),
            "got {err}"
        );
    })
    .await
    .expect("test timed out");
}
