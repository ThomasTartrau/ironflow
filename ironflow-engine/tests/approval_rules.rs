//! Integration tests for dynamic approval rules.
//!
//! Every test drives a real [`Engine`] over a real [`InMemoryStore`] with a real
//! [`WorkflowHandler`]: a shell step runs, then the gate opens and evaluates its
//! rules against the run context. The tests assert on the requirement the
//! engine persisted on the gate.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::task::yield_now;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

use ironflow_core::provider::AgentProvider;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_core::providers::record_replay::RecordReplayProvider;
use ironflow_engine::config::{ApprovalConfig, ApprovalRule, ShellConfig};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::Engine;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_engine::notify::{AuditLogSubscriber, Event};
use ironflow_store::audit_log_store::AuditLogStore;
use ironflow_store::entities::{AuditLogFilter, EventKind};
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{RunStatus, Step, StepKind, TriggerKind};
use ironflow_store::store::{RunStore, Store};

/// Test timeout for bodies that spawn processes.
const TEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Workflow name registered by [`RiskGate`].
const WORKFLOW: &str = "risk-gate";

/// A shell step named `risk`, then an approval gate built from `config`.
struct RiskGate {
    config: ApprovalConfig,
}

impl WorkflowHandler for RiskGate {
    fn name(&self) -> &str {
        WORKFLOW
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.shell("risk", ShellConfig::new("echo high")).await?;
            ctx.approval("finance-gate", self.config.clone()).await?;
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
        .register(RiskGate { config })
        .expect("register handler");
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

fn finance_config() -> ApprovalConfig {
    let rule = ApprovalRule::new("payload.amount > 10000", 2).with_approver_groups(["finance"]);
    ApprovalConfig::new("Release the payment?").with_rule(rule)
}

#[tokio::test]
async fn a_matching_rule_sets_the_requirement() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let engine = engine_with(store.clone(), finance_config());

        let run_id = suspended_run(&engine, json!({"amount": 15000})).await;

        let requirement = gate_step(&store, run_id)
            .await
            .approval_requirement
            .expect("requirement is stored");
        assert_eq!(requirement.rule_index, Some(0));
        assert_eq!(
            requirement.condition.as_deref(),
            Some("payload.amount > 10000")
        );
        assert_eq!(requirement.required_approvers, 2);
        assert_eq!(requirement.approver_groups, vec!["finance".to_string()]);
        assert_eq!(requirement.evaluated.len(), 1);
        assert!(requirement.evaluated[0].matched);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn no_matching_rule_falls_back_to_the_default() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let engine = engine_with(store.clone(), finance_config());

        let run_id = suspended_run(&engine, json!({"amount": 10})).await;

        let requirement = gate_step(&store, run_id)
            .await
            .approval_requirement
            .expect("requirement is stored");
        assert_eq!(requirement.rule_index, None);
        assert_eq!(requirement.condition, None);
        assert_eq!(requirement.required_approvers, 1);
        assert!(requirement.approver_groups.is_empty());
        assert_eq!(requirement.evaluated.len(), 1);
        assert!(!requirement.evaluated[0].matched);
        assert_eq!(requirement.evaluated[0].condition, "payload.amount > 10000");
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn a_rule_can_read_a_previous_step_output() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let config = ApprovalConfig::new("Approve?")
            .with_rule(ApprovalRule::new("steps.risk.output.exit_code == 0", 3))
            .with_rule(ApprovalRule::new("output.exit_code == 0", 2));
        let engine = engine_with(store.clone(), config);

        let run_id = suspended_run(&engine, json!({})).await;

        let requirement = gate_step(&store, run_id)
            .await
            .approval_requirement
            .expect("requirement is stored");
        assert_eq!(requirement.rule_index, Some(0));
        assert_eq!(requirement.required_approvers, 3);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn output_refers_to_the_previous_step() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let config = ApprovalConfig::new("Approve?")
            .with_rule(ApprovalRule::new("output.stdout == 'nope'", 3))
            .with_rule(ApprovalRule::new("output.exit_code == 0", 2));
        let engine = engine_with(store.clone(), config);

        let run_id = suspended_run(&engine, json!({})).await;

        let requirement = gate_step(&store, run_id)
            .await
            .approval_requirement
            .expect("requirement is stored");
        assert_eq!(requirement.rule_index, Some(1));
        assert_eq!(requirement.required_approvers, 2);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn a_config_without_rules_stores_no_requirement() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let engine = engine_with(store.clone(), ApprovalConfig::new("Approve?"));

        let run_id = suspended_run(&engine, json!({"amount": 15000})).await;

        let gate = gate_step(&store, run_id).await;
        assert!(gate.approval_requirement.is_none());
        assert!(gate.approvals.is_empty());
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn approval_requested_audit_entry_carries_the_requirement() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let mut engine = engine_with(store.clone(), finance_config());
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
        assert_eq!(entry.payload["requirement"]["rule_index"], json!(0));
        assert_eq!(entry.payload["requirement"]["required_approvers"], json!(2));
        assert_eq!(
            entry.payload["requirement"]["approver_groups"],
            json!(["finance"])
        );
    })
    .await
    .expect("test timed out");
}
