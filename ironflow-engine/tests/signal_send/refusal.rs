//! A signal the workflow may not send, or a malformed one, fails the step and
//! sends nothing.

use std::sync::Arc;

use tokio::time::timeout;

use ironflow_engine::error::EngineError;
use ironflow_engine::signal::Signal;
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{RunStatus, StepStatus};
use ironflow_store::store::RunStore;

use crate::fixture::{
    ROUTER, RouteReply, TEST_TIMEOUT, Undeclared, engine_with, only_run_of, route, send_steps,
    stored_signals,
};

#[tokio::test]
async fn signal_send_of_an_undeclared_signal_fails_and_sends_nothing() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let router = RouteReply {
            send_undeclared: true,
            ..RouteReply::new()
        };
        let (engine, _seen) = engine_with(&store, router);

        let err = route(&engine, "d-4")
            .await
            .expect_err("an undeclared signal fails the run");
        match &err {
            EngineError::SignalNotAllowed { workflow, signal } => {
                assert_eq!(workflow, ROUTER);
                assert_eq!(signal, Undeclared::NAME);
            }
            other => panic!("expected SignalNotAllowed, got {other:?}"),
        }
        assert!(err.to_string().contains(Undeclared::NAME), "{err}");

        assert_eq!(stored_signals(&store).await, 0);
        let run_id = only_run_of(&store, ROUTER).await;
        let steps = send_steps(&store, run_id).await;
        assert_eq!(steps.len(), 1, "steps: {steps:?}");
        assert_eq!(steps[0].status.state, StepStatus::Failed);
        let error = steps[0].error.as_deref().expect("failed step has an error");
        assert!(error.contains(Undeclared::NAME), "{error}");
        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::Failed);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn signal_send_with_an_empty_or_oversized_key_fails_and_sends_nothing() {
    timeout(TEST_TIMEOUT, async {
        for key in ["  ".to_string(), "k".repeat(256)] {
            let store = Arc::new(InMemoryStore::new());
            let router = RouteReply {
                key: Some(key.clone()),
                ..RouteReply::new()
            };
            let (engine, _seen) = engine_with(&store, router);

            let err = route(&engine, "d-5")
                .await
                .expect_err("an invalid key fails the run");
            assert!(
                matches!(err, EngineError::InvalidSignal(_)),
                "key {key:?}: got {err:?}"
            );
            assert_eq!(stored_signals(&store).await, 0);
            let run_id = only_run_of(&store, ROUTER).await;
            let steps = send_steps(&store, run_id).await;
            assert_eq!(steps.len(), 1);
            assert_eq!(steps[0].status.state, StepStatus::Failed);
        }
    })
    .await
    .expect("test timed out");
}
