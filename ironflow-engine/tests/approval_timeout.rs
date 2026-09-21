//! Integration tests for SLA timers and escalation on approval gates.
//!
//! Every test drives a real [`Engine`] over a real [`InMemoryStore`] with a real
//! [`WorkflowHandler`]: the gate is opened by executing a workflow, the deadline
//! is backdated through the public `update_step` API, and
//! [`ApprovalEscalator::tick`] is called the way the API server's loop calls it.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use chrono::{TimeDelta, Utc};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::time::timeout;
use uuid::Uuid;

use ironflow_core::provider::AgentProvider;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_core::providers::record_replay::RecordReplayProvider;
use ironflow_engine::config::{ApprovalConfig, EscalationPolicy, NotificationTarget, ShellConfig};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::Engine;
use ironflow_engine::escalation::{
    APPROVAL_TIMEOUT_ERROR, ApprovalEscalator, EscalationAction, SYSTEM_TIMEOUT_ACTOR,
};
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_engine::notify::{AuditLogSubscriber, Event};
use ironflow_store::audit_log_store::AuditLogStore;
use ironflow_store::entities::{AuditLogFilter, EventKind};
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{RunStatus, Step, StepKind, StepStatus, StepUpdate, TriggerKind};
use ironflow_store::store::{RunStore, Store};

/// Test timeout for bodies that touch sockets or spawn processes.
const TEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Workflow name registered by [`GatedDeploy`].
const WORKFLOW: &str = "gated-deploy";

/// A deploy pipeline gated on an approval, parameterised by its gate config.
struct GatedDeploy {
    config: ApprovalConfig,
}

impl WorkflowHandler for GatedDeploy {
    fn name(&self) -> &str {
        WORKFLOW
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.shell("build", ShellConfig::new("echo build")).await?;
            ctx.approval("prod-gate", self.config.clone()).await?;
            ctx.shell("deploy", ShellConfig::new("echo deploy")).await?;
            Ok(())
        })
    }
}

fn provider() -> Arc<dyn AgentProvider> {
    Arc::new(RecordReplayProvider::replay(
        ClaudeCodeProvider::new(),
        "/tmp/ironflow-fixtures",
    ))
}

fn engine_with(store: Arc<InMemoryStore>, config: ApprovalConfig) -> Engine {
    let store: Arc<dyn Store> = store;
    let mut engine = Engine::new(store, provider());
    engine
        .register(GatedDeploy { config })
        .expect("register handler");
    engine
}

/// Run the gated workflow until it suspends on the approval gate.
async fn suspended_run(engine: &Engine) -> Uuid {
    let result = engine
        .run_handler(WORKFLOW, TriggerKind::Api, json!({}))
        .await
        .expect("handler suspends on the gate");

    assert_eq!(result.run.status.state, RunStatus::AwaitingApproval);
    result.run.id
}

/// The approval step of a run.
async fn gate_step(store: &Arc<InMemoryStore>, run_id: Uuid) -> Step {
    store
        .list_steps(run_id)
        .await
        .expect("list steps")
        .into_iter()
        .find(|s| s.kind == StepKind::Approval)
        .expect("the run has an approval step")
}

/// Push the gate's deadline into the past so the next tick claims it.
async fn expire_gate(store: &Arc<InMemoryStore>, run_id: Uuid) -> Uuid {
    let step = gate_step(store, run_id).await;
    assert!(
        step.approval_deadline_at.is_some(),
        "the gate must carry a deadline before it can expire"
    );

    store
        .update_step(
            step.id,
            StepUpdate {
                approval_deadline_at: Some(Utc::now() - TimeDelta::seconds(1)),
                ..StepUpdate::default()
            },
        )
        .await
        .expect("backdate the deadline");

    step.id
}

/// Names of the steps recorded for a run.
async fn step_names(store: &Arc<InMemoryStore>, run_id: Uuid) -> Vec<String> {
    store
        .list_steps(run_id)
        .await
        .expect("list steps")
        .into_iter()
        .map(|s| s.name)
        .collect()
}

#[tokio::test]
async fn approval_timeout_auto_approve_completes_step_and_resumes_run() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let config = ApprovalConfig::new("Deploy?")
            .with_deadline_secs(1)
            .on_timeout(EscalationPolicy::AutoApprove);
        let engine = Arc::new(engine_with(store.clone(), config));

        let run_id = suspended_run(&engine).await;
        let step_id = expire_gate(&store, run_id).await;

        let records = ApprovalEscalator::new(engine.clone()).tick().await.expect("tick");

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].action, EscalationAction::Approved);
        assert_eq!(records[0].step_id, step_id);
        assert!(records[0].reason.contains("1s"), "got {}", records[0].reason);

        let gate = store.get_step(step_id).await.unwrap().unwrap();
        assert_eq!(gate.status.state, StepStatus::Completed);
        assert_eq!(
            gate.output.as_ref().unwrap()["approved_by"],
            json!(SYSTEM_TIMEOUT_ACTOR)
        );
        assert!(gate.approval_deadline_at.is_none());

        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::Completed);
        assert!(step_names(&store, run_id).await.contains(&"deploy".to_string()));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn approval_timeout_auto_reject_fails_step_and_run() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let config = ApprovalConfig::new("Deploy?")
            .with_deadline_secs(1)
            .on_timeout(EscalationPolicy::AutoReject);
        let engine = Arc::new(engine_with(store.clone(), config));

        let run_id = suspended_run(&engine).await;
        let step_id = expire_gate(&store, run_id).await;

        let records = ApprovalEscalator::new(engine.clone()).tick().await.expect("tick");

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].action, EscalationAction::Rejected);

        let gate = store.get_step(step_id).await.unwrap().unwrap();
        assert_eq!(gate.status.state, StepStatus::Failed);
        assert_eq!(gate.error.as_deref(), Some(APPROVAL_TIMEOUT_ERROR));
        assert!(gate.approval_deadline_at.is_none());

        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::Failed);
        assert_eq!(run.error.as_deref(), Some(APPROVAL_TIMEOUT_ERROR));
        assert!(!step_names(&store, run_id).await.contains(&"deploy".to_string()));
    })
    .await
    .expect("test timed out");
}

/// Bodies received by the recording webhook server.
type Received = Arc<Mutex<Vec<Value>>>;

async fn record(State(received): State<Received>, Json(body): Json<Value>) -> &'static str {
    received.lock().expect("recorder lock").push(body);
    "ok"
}

/// Spin a real HTTP server that records every posted JSON body.
async fn recording_server() -> (SocketAddr, Received) {
    let received: Received = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new().route("/hook", post(record)).with_state(received.clone());

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    (addr, received)
}

#[tokio::test]
async fn approval_timeout_notify_posts_to_webhook_and_resets_timer() {
    timeout(TEST_TIMEOUT, async {
        let (addr, received) = recording_server().await;
        let store = Arc::new(InMemoryStore::new());
        let config = ApprovalConfig::new("Deploy?")
            .with_deadline_secs(600)
            .on_timeout(EscalationPolicy::Notify(vec![NotificationTarget::Webhook {
                url: format!("http://{addr}/hook"),
            }]));
        let engine = Arc::new(engine_with(store.clone(), config));

        let run_id = suspended_run(&engine).await;
        let step_id = expire_gate(&store, run_id).await;

        let records = ApprovalEscalator::new(engine.clone()).tick().await.expect("tick");

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].action, EscalationAction::Notified(1));

        let bodies = received.lock().expect("recorder lock").clone();
        assert_eq!(bodies.len(), 1, "expected exactly one webhook delivery");
        assert_eq!(bodies[0]["step_name"], json!("prod-gate"));
        assert_eq!(bodies[0]["policy"], json!("notify"));
        assert!(
            bodies[0]["reason"]
                .as_str()
                .expect("reason is a string")
                .contains("600s"),
            "got {}",
            bodies[0]["reason"]
        );

        // The gate stays open and the timer is back in the future.
        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::AwaitingApproval);

        let gate = store.get_step(step_id).await.unwrap().unwrap();
        assert_eq!(gate.status.state, StepStatus::AwaitingApproval);
        assert!(gate.approval_deadline_at.expect("timer rearmed") > Utc::now());
        // A bare Notify repeats: the stage does not advance.
        assert_eq!(gate.approval_stage, 0);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn approval_timeout_escalate_reassigns_and_resets_timer() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let config = ApprovalConfig::new("Deploy?")
            .with_deadline_secs(600)
            .assigned_to("release-managers")
            .on_timeout(EscalationPolicy::Escalate("sre-oncall".to_string()));
        let engine = Arc::new(engine_with(store.clone(), config));

        let run_id = suspended_run(&engine).await;
        let step_id = expire_gate(&store, run_id).await;

        let records = ApprovalEscalator::new(engine.clone()).tick().await.expect("tick");

        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0].action,
            EscalationAction::Reassigned("sre-oncall".to_string())
        );

        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::AwaitingApproval);

        let gate = store.get_step(step_id).await.unwrap().unwrap();
        assert_eq!(gate.status.state, StepStatus::AwaitingApproval);
        assert_eq!(gate.approval_assignee.as_deref(), Some("sre-oncall"));
        assert!(gate.approval_deadline_at.expect("timer rearmed") > Utc::now());
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn approval_timeout_chain_runs_policies_in_order() {
    timeout(TEST_TIMEOUT, async {
        let (addr, received) = recording_server().await;
        let store = Arc::new(InMemoryStore::new());
        let config = ApprovalConfig::new("Deploy?")
            .with_deadline_secs(600)
            .on_timeout(EscalationPolicy::Chain(vec![
                EscalationPolicy::Notify(vec![NotificationTarget::Webhook {
                    url: format!("http://{addr}/hook"),
                }]),
                EscalationPolicy::AutoReject,
            ]));
        let engine = Arc::new(engine_with(store.clone(), config));
        let escalator = ApprovalEscalator::new(engine.clone());

        let run_id = suspended_run(&engine).await;
        let step_id = expire_gate(&store, run_id).await;

        // First expiry: notify and advance to the next link.
        let first = escalator.tick().await.expect("first tick");
        assert_eq!(first[0].action, EscalationAction::Notified(1));
        assert_eq!(first[0].stage, 0);

        let gate = store.get_step(step_id).await.unwrap().unwrap();
        assert_eq!(gate.approval_stage, 1);
        assert_eq!(gate.status.state, StepStatus::AwaitingApproval);
        assert_eq!(received.lock().expect("recorder lock").len(), 1);

        // Second expiry: the chain's terminal link rejects.
        expire_gate(&store, run_id).await;
        let second = escalator.tick().await.expect("second tick");
        assert_eq!(second[0].action, EscalationAction::Rejected);
        assert_eq!(second[0].stage, 1);

        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::Failed);
        assert_eq!(run.error.as_deref(), Some(APPROVAL_TIMEOUT_ERROR));
        // No extra notification for the terminal link.
        assert_eq!(received.lock().expect("recorder lock").len(), 1);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn approval_timeout_chain_exhausted_leaves_the_gate_open() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let config = ApprovalConfig::new("Deploy?")
            .with_deadline_secs(600)
            .on_timeout(EscalationPolicy::Chain(vec![EscalationPolicy::Escalate(
                "sre-oncall".to_string(),
            )]));
        let engine = Arc::new(engine_with(store.clone(), config));
        let escalator = ApprovalEscalator::new(engine.clone());

        let run_id = suspended_run(&engine).await;
        let step_id = expire_gate(&store, run_id).await;

        escalator.tick().await.expect("first tick");
        expire_gate(&store, run_id).await;
        let second = escalator.tick().await.expect("second tick");

        assert_eq!(second[0].action, EscalationAction::Exhausted);

        // The gate is still open, with no timer left, and never auto-rejected.
        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::AwaitingApproval);
        let gate = store.get_step(step_id).await.unwrap().unwrap();
        assert_eq!(gate.status.state, StepStatus::AwaitingApproval);
        assert!(gate.approval_deadline_at.is_none());
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn approval_timeout_without_deadline_never_fires() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let engine = Arc::new(engine_with(store.clone(), ApprovalConfig::new("Deploy?")));

        let run_id = suspended_run(&engine).await;
        let gate = gate_step(&store, run_id).await;
        assert!(gate.approval_deadline_at.is_none());

        let records = ApprovalEscalator::new(engine.clone()).tick().await.expect("tick");

        assert!(records.is_empty());
        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::AwaitingApproval);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn approval_timeout_ignores_a_gate_approved_in_the_meantime() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let config = ApprovalConfig::new("Deploy?")
            .with_deadline_secs(600)
            .on_timeout(EscalationPolicy::AutoReject);
        let engine = Arc::new(engine_with(store.clone(), config));

        let run_id = suspended_run(&engine).await;
        let step_id = expire_gate(&store, run_id).await;

        // A human approves between the deadline passing and the tick.
        store
            .update_run_status(run_id, RunStatus::Running)
            .await
            .expect("approve");

        let records = ApprovalEscalator::new(engine.clone()).tick().await.expect("tick");

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].action, EscalationAction::Stale);

        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::Running);
        let gate = store.get_step(step_id).await.unwrap().unwrap();
        assert_eq!(gate.status.state, StepStatus::AwaitingApproval);
        assert!(gate.error.is_none());
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn approval_timeout_survives_a_restart() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let config = ApprovalConfig::new("Deploy?")
            .with_deadline_secs(600)
            .on_timeout(EscalationPolicy::AutoReject);

        let run_id = {
            let engine = Arc::new(engine_with(store.clone(), config.clone()));
            let run_id = suspended_run(&engine).await;
            expire_gate(&store, run_id).await;
            run_id
            // The engine (and its escalator) go away here.
        };

        // A brand-new process over the same store still fires the timer.
        let engine = Arc::new(engine_with(store.clone(), config));
        let records = ApprovalEscalator::new(engine).tick().await.expect("tick");

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].action, EscalationAction::Rejected);

        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::Failed);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn approval_timeout_legacy_timeout_seconds_auto_rejects() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let config = ApprovalConfig::new("Deploy?").with_timeout_seconds(3600);
        let engine = Arc::new(engine_with(store.clone(), config));

        let run_id = suspended_run(&engine).await;
        let step_id = expire_gate(&store, run_id).await;

        let records = ApprovalEscalator::new(engine.clone()).tick().await.expect("tick");

        assert_eq!(records[0].action, EscalationAction::Rejected);
        assert!(
            records[0].reason.contains("3600s"),
            "got {}",
            records[0].reason
        );

        let gate = store.get_step(step_id).await.unwrap().unwrap();
        assert_eq!(gate.status.state, StepStatus::Failed);
        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::Failed);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn approval_timeout_writes_an_audit_entry_with_the_reason() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let config = ApprovalConfig::new("Deploy?")
            .with_deadline_secs(900)
            .on_timeout(EscalationPolicy::AutoReject);

        let store_dyn: Arc<dyn Store> = store.clone();
        let mut engine = Engine::new(store_dyn, provider());
        engine
            .register(GatedDeploy { config })
            .expect("register handler");
        engine.subscribe(AuditLogSubscriber::new(store.clone()), Event::ALL);
        let engine = Arc::new(engine);

        let run_id = suspended_run(&engine).await;
        expire_gate(&store, run_id).await;

        ApprovalEscalator::new(engine.clone()).tick().await.expect("tick");

        // Subscribers run in spawned tasks; give them a turn to persist.
        tokio::task::yield_now().await;
        tokio::time::sleep(Duration::from_millis(50)).await;

        let page = store
            .list_audit_logs(
                AuditLogFilter {
                    event_type: Some(EventKind::ApprovalEscalated),
                    run_id: Some(run_id),
                    from: None,
                    to: None,
                },
                1,
                50,
            )
            .await
            .expect("list audit logs");

        assert_eq!(page.items.len(), 1, "expected one escalation audit entry");
        let entry = &page.items[0];
        assert_eq!(entry.event_type, EventKind::ApprovalEscalated);
        assert_eq!(entry.run_id, Some(run_id));
        assert!(entry.step_id.is_some());
        assert_eq!(entry.payload["action"], json!("rejected"));
        assert_eq!(entry.payload["policy"], json!("auto_reject"));
        assert!(
            entry.payload["reason"]
                .as_str()
                .expect("reason is a string")
                .contains("900s"),
            "got {}",
            entry.payload["reason"]
        );
    })
    .await
    .expect("test timed out");
}
