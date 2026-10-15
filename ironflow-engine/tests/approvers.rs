//! Integration tests for approval gates that require several approvers.
//!
//! Every test drives a real [`Engine`] over a real [`InMemoryStore`] with a real
//! [`WorkflowHandler`]: the handler reads its typed input, runs a shell step,
//! computes the [`Approvers`] in plain Rust and opens the gate. The tests assert
//! on the requirement the engine persisted on the gate.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use chrono::Utc;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::task::yield_now;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

use ironflow_core::provider::AgentProvider;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_core::providers::record_replay::RecordReplayProvider;
use ironflow_engine::config::{ApprovalConfig, Approvers, ShellConfig};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::Engine;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_engine::notify::{AuditLogSubscriber, Event};
use ironflow_store::audit_log_store::AuditLogStore;
use ironflow_store::entities::{ApprovalRequirement, AuditLogFilter, EventKind};
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{RunStatus, Step, StepApproval, StepKind, StepStatus, TriggerKind};
use ironflow_store::store::{RunStore, Store};

/// Test timeout for bodies that spawn processes.
const TEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Workflow name registered by [`PaymentGate`].
const WORKFLOW: &str = "payment-gate";

#[derive(Deserialize)]
struct Payment {
    amount: u64,
}

/// How the gate decides its approvers.
enum Policy {
    /// The payment matrix of the issue: by amount.
    ByAmount,
    /// From the output of the `risk` shell step.
    ByRisk,
    /// No `requiring` call at all.
    Unset,
    /// Two approvers on the first execution, five on every later one.
    ChangesOnReplay(AtomicU32),
}

/// A shell step named `risk`, then an approval gate whose approvers follow
/// `policy`.
struct PaymentGate {
    policy: Policy,
}

impl WorkflowHandler for PaymentGate {
    fn name(&self) -> &str {
        WORKFLOW
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let payment: Payment = ctx.input().await?;
            let risk = ctx.shell("risk", ShellConfig::new("echo high")).await?;

            let config = ApprovalConfig::new("Release the payment?");
            let config = match &self.policy {
                Policy::ByAmount => config.requiring(match payment.amount {
                    a if a > 100_000 => Approvers::at_least(3)
                        .from_groups(["finance", "board"])
                        .because("amount > 100k"),
                    a if a > 10_000 => Approvers::at_least(2)
                        .from_groups(["finance"])
                        .because("amount > 10k"),
                    _ => Approvers::any(),
                }),
                Policy::ByRisk => config.requiring(if risk.stdout().trim() == "high" {
                    Approvers::at_least(3).because("high risk")
                } else {
                    Approvers::any()
                }),
                Policy::Unset => config,
                Policy::ChangesOnReplay(executions) => {
                    let n = if executions.fetch_add(1, Ordering::SeqCst) == 0 {
                        2
                    } else {
                        5
                    };
                    config.requiring(Approvers::at_least(n).because("first execution wins"))
                }
            };

            ctx.approval("finance-gate", config).await?;
            Ok(())
        })
    }
}

/// Workflow name registered by [`GateThenShell`] with two required approvers.
const TWO_APPROVERS: &str = "two-approver-gate";

/// Workflow name registered by [`GateThenShell`] without approvers.
const ANY_APPROVER: &str = "any-approver-gate";

/// Name of the step [`GateThenShell`] runs past its gate.
const AFTER_GATE: &str = "after-gate";

/// An approval gate with an SLA deadline, then a shell step.
struct GateThenShell {
    name: &'static str,
    required: Option<u32>,
}

impl WorkflowHandler for GateThenShell {
    fn name(&self) -> &str {
        self.name
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let mut config = ApprovalConfig::new("Ship it?").with_deadline_secs(3600);
            if let Some(required) = self.required {
                config = config.requiring(Approvers::at_least(required));
            }
            ctx.approval("gate", config).await?;
            ctx.shell(AFTER_GATE, ShellConfig::new("echo shipped"))
                .await?;
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

fn engine_with(store: Arc<InMemoryStore>, policy: Policy) -> Engine {
    let store: Arc<dyn Store> = store;
    let mut engine = Engine::new(store, provider());
    engine
        .register(PaymentGate { policy })
        .expect("register handler");
    for (name, required) in [(TWO_APPROVERS, Some(2)), (ANY_APPROVER, None)] {
        engine
            .register(GateThenShell { name, required })
            .expect("register gated handler");
    }
    engine
}

/// Run the workflow with `payload` until it suspends on the gate.
async fn suspended_run(engine: &Engine, payload: Value) -> Uuid {
    let result = engine
        .run_handler(WORKFLOW, TriggerKind::Api, payload)
        .await
        .expect("handler suspends on the gate");

    assert_eq!(result.run.status.state, RunStatus::AwaitingApproval);
    result.run.id
}

/// The approval step of a run.
async fn gate_step(store: &InMemoryStore, run_id: Uuid) -> Step {
    store
        .list_steps(run_id)
        .await
        .expect("list steps")
        .into_iter()
        .find(|s| s.kind == StepKind::Approval)
        .expect("approval step exists")
}

/// Record a vote of a new user on the gate `gate_id`.
async fn vote(store: &InMemoryStore, gate_id: Uuid) {
    store
        .record_step_approval(
            gate_id,
            StepApproval {
                user_id: Uuid::now_v7(),
                approved_by: "tester".to_string(),
                at: Utc::now(),
            },
        )
        .await
        .expect("record vote");
}

/// Move a suspended run back to `Running` and resume it, as an approval
/// outside the API would.
async fn resume(engine: &Engine, store: &InMemoryStore, run_id: Uuid) -> RunStatus {
    store
        .update_run_status(run_id, RunStatus::Running)
        .await
        .expect("to running");
    let resumed = engine.resume_run(run_id).await.expect("resume");
    resumed.run.status.state
}

async fn stored_requirement(store: &InMemoryStore, run_id: Uuid) -> ApprovalRequirement {
    gate_step(store, run_id)
        .await
        .approval_requirement
        .expect("requirement is stored")
}

#[tokio::test]
async fn a_mid_sized_payment_needs_two_finance_approvers() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let engine = engine_with(store.clone(), Policy::ByAmount);

        let run_id = suspended_run(&engine, json!({"amount": 15000})).await;

        assert_eq!(
            stored_requirement(&store, run_id).await,
            ApprovalRequirement {
                reason: Some("amount > 10k".to_string()),
                required_approvers: 2,
                approver_groups: vec!["finance".to_string()],
            }
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn a_large_payment_needs_three_approvers_from_finance_or_the_board() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let engine = engine_with(store.clone(), Policy::ByAmount);

        let run_id = suspended_run(&engine, json!({"amount": 500000})).await;

        let requirement = stored_requirement(&store, run_id).await;
        assert_eq!(requirement.reason.as_deref(), Some("amount > 100k"));
        assert_eq!(requirement.required_approvers, 3);
        assert_eq!(requirement.approver_groups, vec!["finance", "board"]);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn any_approver_stores_the_default_requirement() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let engine = engine_with(store.clone(), Policy::ByAmount);

        let run_id = suspended_run(&engine, json!({"amount": 10})).await;

        assert_eq!(
            stored_requirement(&store, run_id).await,
            ApprovalRequirement::default()
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn approvers_can_follow_a_previous_step_output() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let engine = engine_with(store.clone(), Policy::ByRisk);

        let run_id = suspended_run(&engine, json!({"amount": 10})).await;

        let requirement = stored_requirement(&store, run_id).await;
        assert_eq!(requirement.reason.as_deref(), Some("high risk"));
        assert_eq!(requirement.required_approvers, 3);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn a_gate_without_requiring_stores_no_requirement() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let engine = engine_with(store.clone(), Policy::Unset);

        let run_id = suspended_run(&engine, json!({"amount": 15000})).await;

        let gate = gate_step(&store, run_id).await;
        assert!(gate.approval_requirement.is_none());
        assert!(gate.approvals.is_empty());
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn a_resumed_run_keeps_the_requirement_stored_when_the_gate_opened() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let engine = engine_with(store.clone(), Policy::ChangesOnReplay(AtomicU32::new(0)));

        let run_id = suspended_run(&engine, json!({"amount": 15000})).await;
        assert_eq!(
            stored_requirement(&store, run_id).await.required_approvers,
            2
        );

        // The approval API records the votes, then moves the run back to
        // Running before resuming.
        let gate = gate_step(&store, run_id).await;
        vote(&store, gate.id).await;
        vote(&store, gate.id).await;
        store
            .update_run_status(run_id, RunStatus::Running)
            .await
            .expect("to running");
        let resumed = engine.resume_run(run_id).await.expect("resume");

        // The handler computed five approvers on replay; the gate kept two.
        assert_eq!(resumed.run.status.state, RunStatus::Completed);
        assert_eq!(
            stored_requirement(&store, run_id).await.required_approvers,
            2
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn approval_requested_audit_entry_carries_the_requirement() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let mut engine = engine_with(store.clone(), Policy::ByAmount);
        engine.subscribe(AuditLogSubscriber::new(store.clone()), Event::ALL);

        let run_id = suspended_run(&engine, json!({"amount": 15000})).await;
        let gate = gate_step(&store, run_id).await;

        // Subscribers run in spawned tasks; give them a turn to persist.
        yield_now().await;
        sleep(Duration::from_millis(50)).await;

        let page = store
            .list_audit_logs(
                AuditLogFilter {
                    event_type: Some(EventKind::ApprovalRequested),
                    run_id: Some(run_id),
                    from: None,
                    to: None,
                },
                1,
                50,
            )
            .await
            .expect("list audit logs");

        assert_eq!(page.items.len(), 1, "expected one approval_requested entry");
        let entry = &page.items[0];
        assert_eq!(entry.step_id, Some(gate.id));
        assert_eq!(entry.payload["message"], json!("Release the payment?"));
        assert_eq!(
            entry.payload["requirement"],
            json!({
                "reason": "amount > 10k",
                "required_approvers": 2,
                "approver_groups": ["finance"],
            })
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn a_replay_with_missing_votes_does_not_auto_approve_the_gate() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let engine = engine_with(store.clone(), Policy::Unset);
        let result = engine
            .run_handler(TWO_APPROVERS, TriggerKind::Api, json!({}))
            .await
            .expect("handler suspends on the gate");
        let run_id = result.run.id;
        let gate = gate_step(&store, run_id).await;
        let deadline = gate.approval_deadline_at.expect("the gate has a deadline");

        vote(&store, gate.id).await;
        let status = resume(&engine, &store, run_id).await;

        assert_eq!(status, RunStatus::AwaitingApproval);
        let steps = store.list_steps(run_id).await.expect("list steps");
        let gates: Vec<&Step> = steps
            .iter()
            .filter(|s| s.kind == StepKind::Approval)
            .collect();
        assert_eq!(gates.len(), 1, "the replay records no second gate");
        assert_eq!(gates[0].status.state, StepStatus::AwaitingApproval);
        assert_eq!(gates[0].approval_deadline_at, Some(deadline));
        assert_eq!(gates[0].approvals.len(), 1);
        assert!(steps.iter().all(|s| s.name != AFTER_GATE));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn a_replayed_gate_without_any_vote_is_not_auto_approved() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let engine = engine_with(store.clone(), Policy::Unset);
        let result = engine
            .run_handler(ANY_APPROVER, TriggerKind::Api, json!({}))
            .await
            .expect("handler suspends on the gate");
        let run_id = result.run.id;

        let status = resume(&engine, &store, run_id).await;

        assert_eq!(status, RunStatus::AwaitingApproval);
        let gate = gate_step(&store, run_id).await;
        assert_eq!(gate.status.state, StepStatus::AwaitingApproval);
        assert!(gate.approval_deadline_at.is_some());
        let steps = store.list_steps(run_id).await.expect("list steps");
        assert_eq!(steps.len(), 1);
        assert!(steps.iter().all(|s| s.name != AFTER_GATE));

        vote(&store, gate.id).await;
        let status = resume(&engine, &store, run_id).await;

        assert_eq!(status, RunStatus::Completed);
        let steps = store.list_steps(run_id).await.expect("list steps");
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].status.state, StepStatus::Completed);
        assert!(steps[0].approval_deadline_at.is_none());
        assert_eq!(steps[1].name, AFTER_GATE);
        assert_eq!(steps[1].status.state, StepStatus::Completed);
    })
    .await
    .expect("test timed out");
}
