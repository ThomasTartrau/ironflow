//! A send step executed again never delivers twice: replayed once completed,
//! deduplicated when it was interrupted mid-delivery.

use std::future::pending;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::json;
use tokio::spawn;
use tokio::sync::Notify;
use tokio::time::{sleep, timeout};

use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::Engine;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_engine::signal::{SentSignals, SignalOrigin, SignalSendFuture, SignalSender};
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{LeaseRequest, NewSignal, RunStatus, TriggerKind};
use ironflow_store::store::{RunStore, STEP_INTERRUPTED_ERROR};

use crate::fixture::{
    HumanReplied, ROUTER, RouteReply, TEST_TIMEOUT, Undeclared, provider, send_steps,
    stored_signals, wait_for_status,
};

/// Sends, then waits for a signal of its own: it suspends, and is replayed
/// from the start on resume.
struct SendThenWait;

impl WorkflowHandler for SendThenWait {
    fn name(&self) -> &str {
        "send-then-wait"
    }

    fn sent_signals(&self) -> SentSignals {
        SentSignals::new().allow::<HumanReplied>()
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let reply = HumanReplied {
                body: "first".to_string(),
            };
            ctx.send_signal("notify", &reply, "elsewhere").await?;
            ctx.wait_for_signal::<Undeclared>("wait", "me", Duration::from_secs(3600))
                .await?;
            Ok(())
        })
    }
}

#[tokio::test]
async fn signal_send_is_replayed_without_sending_again_on_resume() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let mut engine = Engine::new(store.clone(), provider());
        engine.register(SendThenWait).expect("register");
        let engine = Arc::new(engine);

        let result = engine
            .run_handler("send-then-wait", TriggerKind::Manual, json!({}))
            .await
            .expect("suspends on its wait");
        assert_eq!(result.run.status.state, RunStatus::Sleeping);
        assert_eq!(stored_signals(&store).await, 1);

        let wake = Undeclared {
            body: "go".to_string(),
        };
        engine
            .send_signal(&wake, "me", None)
            .await
            .expect("wake it");
        wait_for_status(&store, result.run.id, RunStatus::Completed).await;

        // One `HumanReplied` from the workflow, one `Undeclared` from the test.
        assert_eq!(stored_signals(&store).await, 2);
        assert_eq!(send_steps(&store, result.run.id).await.len(), 1);
    })
    .await
    .expect("test timed out");
}

/// Delivers in-process, then hangs while `hang` is set: the worker dies after
/// the API stored the signal, before it recorded the answer.
struct HangAfterDelivery {
    engine: Arc<Engine>,
    hang: Arc<AtomicBool>,
    delivered: Arc<Notify>,
}

impl SignalSender for HangAfterDelivery {
    fn send(&self, signal: NewSignal, origin: SignalOrigin) -> SignalSendFuture<'_> {
        Box::pin(async move {
            let delivery = self
                .engine
                .deliver_signal_from(signal, Some(origin))
                .await?;
            if self.hang.load(Ordering::SeqCst) {
                self.delivered.notify_one();
                pending::<()>().await;
            }
            Ok(delivery)
        })
    }
}

#[tokio::test]
async fn signal_send_interrupted_and_executed_again_does_not_send_twice() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let hang = Arc::new(AtomicBool::new(true));
        let delivered = Arc::new(Notify::new());
        let sender = HangAfterDelivery {
            engine: Arc::new(Engine::new(store.clone(), provider())),
            hang: hang.clone(),
            delivered: delivered.clone(),
        };
        let router = RouteReply::new();
        let deliveries = router.deliveries.clone();
        let mut engine =
            Engine::new(store.clone(), provider()).with_signal_sender(Arc::new(sender));
        engine.register(router).expect("register router");
        let engine = Arc::new(engine);

        let run_id = engine
            .enqueue_handler(
                ROUTER,
                TriggerKind::Manual,
                json!({ "discussion_id": "d-7" }),
                3,
            )
            .await
            .expect("enqueue")
            .id;
        store
            .pick_next_pending(Some(LeaseRequest {
                worker_id: "worker-that-dies".to_string(),
                ttl: Duration::from_millis(1),
            }))
            .await
            .expect("pick")
            .expect("a pending run");

        let worker = engine.clone();
        let task = spawn(async move { worker.execute_handler_run(run_id).await });
        delivered.notified().await;
        task.abort();
        assert!(
            task.await
                .expect_err("the worker was killed")
                .is_cancelled()
        );
        assert_eq!(stored_signals(&store).await, 1);

        // Reap the lease and clean the steps up like the API's reaper.
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

        hang.store(false, Ordering::SeqCst);
        store
            .pick_next_pending(None)
            .await
            .expect("pick")
            .expect("the requeued run");
        engine
            .execute_handler_run(run_id)
            .await
            .expect("the requeued run completes");

        assert_eq!(stored_signals(&store).await, 1, "sent once");
        let steps = send_steps(&store, run_id).await;
        assert_eq!(steps.len(), 2, "steps: {steps:?}");
        assert_eq!(steps[0].position, steps[1].position);
        assert!(
            steps
                .iter()
                .any(|s| s.error.as_deref() == Some(STEP_INTERRUPTED_ERROR))
        );
        let deliveries = deliveries.lock().unwrap();
        assert_eq!(deliveries.len(), 1);
        assert!(
            deliveries[0].duplicate,
            "the second delivery is a duplicate"
        );
    })
    .await
    .expect("test timed out");
}
