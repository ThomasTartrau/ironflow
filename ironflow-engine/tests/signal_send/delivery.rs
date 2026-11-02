//! A declared signal is delivered: the waiting run resumes, the send shows in
//! the sender's timeline and in the audit log.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{from_value, json};
use tokio::time::{sleep, timeout};

use ironflow_engine::plan::PlanOptions;
use ironflow_engine::signal::{Signal, SignalDelivery};
use ironflow_store::audit_log_store::AuditLogStore;
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{
    AuditLogFilter, EventKind, RunStatus, SignalFilter, StepKind, StepStatus,
};
use ironflow_store::signal_store::SignalStore;

use crate::fixture::{
    HumanReplied, ROUTER, RouteReply, TEST_TIMEOUT, engine_with, route, send_steps, start_waiting,
    stored_signals, wait_for_status,
};

#[tokio::test]
async fn signal_send_resumes_the_waiting_run_without_api_key() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let router = RouteReply::new();
        let deliveries = router.deliveries.clone();
        let (engine, seen) = engine_with(&store, router);
        let engine = Arc::new(engine);
        let waiting = start_waiting(&engine, "d-1").await;

        let router_run = route(&engine, "d-1").await.expect("router completes");
        wait_for_status(&store, router_run, RunStatus::Completed).await;

        wait_for_status(&store, waiting, RunStatus::Completed).await;
        assert_eq!(*seen.lock().unwrap(), vec!["ship it".to_string()]);
        let deliveries = deliveries.lock().unwrap();
        assert_eq!(deliveries.len(), 1);
        assert!(!deliveries[0].duplicate);
        assert!(deliveries[0].rejected.is_empty());
        assert_eq!(deliveries[0].resumed.len(), 1);
        assert_eq!(deliveries[0].resumed[0].run_id, waiting);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn signal_send_records_a_step_in_the_sender_timeline() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let (engine, _seen) = engine_with(&store, RouteReply::new());
        let engine = Arc::new(engine);
        let waiting = start_waiting(&engine, "d-2").await;

        let router_run = route(&engine, "d-2").await.expect("router completes");

        let steps = send_steps(&store, router_run).await;
        assert_eq!(steps.len(), 1, "steps: {steps:?}");
        let step = &steps[0];
        assert_eq!(step.name, "notify");
        assert_eq!(step.status.state, StepStatus::Completed);
        let input = step.input.as_ref().expect("send step records its input");
        assert_eq!(input["name"], json!(HumanReplied::NAME));
        assert_eq!(input["key"], json!("d-2"));
        assert_eq!(input["payload"], json!({ "body": "ship it" }));
        let delivery: SignalDelivery =
            from_value(step.output.clone().expect("send step has an output"))
                .expect("output is a delivery");
        assert!(!delivery.duplicate);
        assert_eq!(delivery.resumed.len(), 1);
        assert_eq!(delivery.resumed[0].run_id, waiting);
        let stored = store
            .list_signals(SignalFilter::default(), 1, 20)
            .await
            .expect("list signals");
        assert_eq!(stored.items.len(), 1);
        assert_eq!(delivery.signal_id, stored.items[0].id);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn signal_send_audit_entry_names_the_sending_run_and_step() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let (engine, _seen) = engine_with(&store, RouteReply::new());
        let engine = Arc::new(engine);
        start_waiting(&engine, "d-3").await;

        let router_run = route(&engine, "d-3").await.expect("router completes");
        let step = send_steps(&store, router_run).await.remove(0);

        let filter = AuditLogFilter {
            event_type: Some(EventKind::SignalReceived),
            run_id: Some(router_run),
            ..AuditLogFilter::default()
        };
        // The audit subscriber runs on its own task.
        let entry = loop {
            let page = store
                .list_audit_logs(filter.clone(), 1, 20)
                .await
                .expect("list audit logs");
            if let Some(entry) = page.items.into_iter().next() {
                break entry;
            }
            sleep(Duration::from_millis(20)).await;
        };
        assert_eq!(entry.run_id, Some(router_run));
        assert_eq!(entry.step_id, Some(step.id));
        assert_eq!(entry.payload["sender_run_id"], json!(router_run));
        assert_eq!(entry.payload["sender_step_id"], json!(step.id));
        assert_eq!(entry.payload["name"], json!(HumanReplied::NAME));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn signal_send_while_planning_records_the_step_and_sends_nothing() {
    let store = Arc::new(InMemoryStore::new());
    let (engine, _seen) = engine_with(&store, RouteReply::new());

    let plan = engine
        .plan_handler(
            ROUTER,
            json!({ "discussion_id": "d-8" }),
            PlanOptions::default(),
        )
        .await
        .expect("plan");

    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].name, "notify");
    assert_eq!(plan.steps[0].kind, StepKind::SignalSend);
    assert_eq!(stored_signals(&store).await, 0);
}
