//! End-to-end tests for `ctx.decision(...)`.
//!
//! Black-box: a real [`Engine`] with an [`InMemoryStore`] and a
//! [`RecordReplayDecisionProvider`] replaying a real Jev response. The fixture in
//! `tests/fixtures/decisions/triage-output.json` was captured from the live
//! TypeSafe model through OpenRouter (`typesafe/jev-1.13`); refresh it with the
//! `record_triage_fixture_from_openrouter` test in `ironflow-core`. No mocks.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use rust_decimal::Decimal;
use serde_json::json;

use ironflow_core::decision::DecisionOutput;
use ironflow_core::provider::AgentProvider;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_core::providers::record_replay::RecordReplayProvider;
use ironflow_core::providers::record_replay_decision::{
    RecordReplayDecisionProvider, hash_request,
};
use ironflow_engine::config::{DecisionConfig, ShellConfig};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::Engine;
use ironflow_engine::error::EngineError;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{RunStatus, StepKind, StepStatus, TriggerKind};

const STATE: &str = "Help! My payouts have been failing for 3 days.";

/// The questions under test. `escalate_below` does not affect the request hash,
/// so the same fixture serves both the happy-path and the escalation config.
fn build_config(escalate_below: Option<f64>) -> DecisionConfig {
    let mut config = DecisionConfig::new(STATE)
        .noul("is_urgent", "Does this convey urgency?")
        .choice(
            "department",
            "Which team?",
            &["billing", "technical", "sales"],
        )
        .score(
            "frustration",
            "How frustrated?",
            &["Calm", "Frustrated", "Very angry"],
        );
    if let Some(t) = escalate_below {
        config = config.escalate_below(t);
    }
    config
}

/// The real Jev response captured from OpenRouter for the triage request.
///
/// Recorded answers: `is_urgent` noul 0.95 (derived confidence 0.90), `department`
/// choice "billing" (confidence 0.73), `frustration` score 1.03 (confidence 0.94),
/// usage 353 input tokens. The lowest confidence is therefore 0.73, which drives
/// the escalation tests (threshold above 0.73 escalates, below does not).
fn real_triage_output() -> DecisionOutput {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/decisions/triage-output.json"
    );
    let json = fs::read_to_string(path).expect("recorded triage fixture must exist");
    serde_json::from_str(&json).expect("recorded fixture parses into DecisionOutput")
}

struct TempDir(String);
impl Drop for TempDir {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).ok();
    }
}

fn temp_fixtures() -> (String, TempDir) {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let dir = format!(
        "/tmp/ironflow-decision-e2e-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    fs::create_dir_all(&dir).unwrap();
    (dir.clone(), TempDir(dir))
}

fn write_fixture(dir: &str, config: &DecisionConfig, output: &DecisionOutput) {
    let request = config.to_request();
    let hash = hash_request(&request);
    let fixture = json!({ "request": request, "output": output });
    let path = PathBuf::from(dir).join(format!("{hash}.json"));
    fs::write(path, serde_json::to_string_pretty(&fixture).unwrap()).unwrap();
}

fn agent_provider() -> Arc<dyn AgentProvider> {
    Arc::new(RecordReplayProvider::replay(
        ClaudeCodeProvider::new(),
        "/tmp/ironflow-fixtures",
    ))
}

/// Handler: one decision step, then a shell step that only runs if the decision
/// returned (i.e. did not escalate).
struct TriageWorkflow {
    escalate_below: Option<f64>,
}

impl WorkflowHandler for TriageWorkflow {
    fn name(&self) -> &str {
        "triage"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let out = ctx
                .decision("triage", build_config(self.escalate_below))
                .await?;
            // Exercise the typed accessors inside the handler.
            let _ = out.noul("is_urgent")?;
            let team = out.choice("department")?.choice.clone();
            let cmd = format!("echo routed to {team}");
            ctx.shell("route", ShellConfig::new(&cmd)).await?;
            Ok(())
        })
    }
}

#[tokio::test]
async fn decision_happy_path_completes_and_exposes_typed_answers() {
    let (dir, _guard) = temp_fixtures();
    // Threshold below the real min confidence (0.73): the decision returns.
    let config = build_config(Some(0.5));
    write_fixture(&dir, &config, &real_triage_output());

    let store = Arc::new(InMemoryStore::new());
    let mut engine = Engine::new(store, agent_provider())
        .with_decision_provider(Arc::new(RecordReplayDecisionProvider::replay(&dir)));
    engine
        .register(TriageWorkflow {
            escalate_below: Some(0.5),
        })
        .unwrap();

    let result = engine
        .run_handler("triage", TriggerKind::Api, json!({}))
        .await
        .unwrap();
    assert_eq!(result.run.status.state, RunStatus::Completed);

    let steps = engine.store().list_steps(result.run.id).await.unwrap();
    let decision = steps.iter().find(|s| s.name == "triage").unwrap();
    assert_eq!(decision.kind, StepKind::Decision);
    assert_eq!(decision.status.state, StepStatus::Completed);
    let stored: DecisionOutput = serde_json::from_value(decision.output.clone().unwrap()).unwrap();
    assert_eq!(stored.choice("department").unwrap().choice, "billing");

    // The routing step ran, proving the decision returned Ok.
    assert!(steps.iter().any(|s| s.name == "route"));
}

#[tokio::test]
async fn decision_escalates_below_threshold_then_replays_on_resume() {
    let (dir, _guard) = temp_fixtures();
    // Threshold above the real min confidence (0.73): the decision escalates.
    let config = build_config(Some(0.9));
    write_fixture(&dir, &config, &real_triage_output());

    let store = Arc::new(InMemoryStore::new());
    let mut engine = Engine::new(store, agent_provider())
        .with_decision_provider(Arc::new(RecordReplayDecisionProvider::replay(&dir)));
    engine
        .register(TriageWorkflow {
            escalate_below: Some(0.9),
        })
        .unwrap();

    // First execution escalates: run suspends, routing step does not run.
    let result = engine
        .run_handler("triage", TriggerKind::Api, json!({}))
        .await
        .unwrap();
    assert_eq!(result.run.status.state, RunStatus::AwaitingApproval);

    let steps = engine.store().list_steps(result.run.id).await.unwrap();
    let decision = steps.iter().find(|s| s.name == "triage").unwrap();
    assert_eq!(decision.status.state, StepStatus::AwaitingApproval);
    assert!(!steps.iter().any(|s| s.name == "route"));

    // Approve: the approval API moves the run back to Running before resuming
    // (a run cannot transition AwaitingApproval -> Completed directly).
    engine
        .store()
        .update_run_status(result.run.id, RunStatus::Running)
        .await
        .unwrap();

    // Resume after approval: the decision is replayed (not re-called) and the run
    // completes.
    let resumed = engine.resume_run(result.run.id).await.unwrap();
    assert_eq!(resumed.run.status.state, RunStatus::Completed);
    // The decision cost is counted once, not double-charged on replay: getting the
    // single real cost (not twice it) is what proves the replay path was taken.
    assert_eq!(resumed.run.cost_usd, real_triage_output().usage.cost_usd());

    let steps = engine.store().list_steps(result.run.id).await.unwrap();
    let decision = steps.iter().find(|s| s.name == "triage").unwrap();
    assert_eq!(decision.status.state, StepStatus::Completed);
    let stored: DecisionOutput = serde_json::from_value(decision.output.clone().unwrap()).unwrap();
    assert_eq!(stored.choice("department").unwrap().choice, "billing");
    assert!(steps.iter().any(|s| s.name == "route"));
}

#[tokio::test]
async fn decision_without_provider_fails_explicitly() {
    let store = Arc::new(InMemoryStore::new());
    // No decision provider wired.
    let mut engine = Engine::new(store, agent_provider());
    engine
        .register(TriageWorkflow {
            escalate_below: None,
        })
        .unwrap();

    // A missing provider is a deterministic, explicit failure: the handler errors
    // and the run is persisted as Failed (run_handler propagates the error).
    let err = engine
        .run_handler("triage", TriggerKind::Api, json!({}))
        .await
        .unwrap_err();
    assert!(
        matches!(err, EngineError::NoDecisionProvider { .. }),
        "expected NoDecisionProvider, got: {err:?}"
    );
    assert!(err.to_string().contains("decision provider"));
}

#[tokio::test]
async fn decision_cost_is_imputed_to_run_budget() {
    let (dir, _guard) = temp_fixtures();
    let config = build_config(Some(0.5));
    write_fixture(&dir, &config, &real_triage_output());

    let store = Arc::new(InMemoryStore::new());
    let mut engine = Engine::new(store, agent_provider())
        .with_decision_provider(Arc::new(RecordReplayDecisionProvider::replay(&dir)));
    engine
        .register(TriageWorkflow {
            escalate_below: Some(0.5),
        })
        .unwrap();

    let result = engine
        .run_handler("triage", TriggerKind::Api, json!({}))
        .await
        .unwrap();
    assert_eq!(result.run.status.state, RunStatus::Completed);
    // Cost is imputed from the real usage (353 input tokens at the Jev rate).
    let expected = real_triage_output().usage.cost_usd();
    assert!(expected > Decimal::ZERO);
    assert_eq!(result.run.cost_usd, expected);
}

/// Guard against an accidental future where `ApprovalRequired` maps to a
/// non-error status (compile-time proof that the variant exists).
#[allow(dead_code)]
fn approval_required_is_an_error(e: EngineError) -> bool {
    matches!(e, EngineError::ApprovalRequired { .. })
}
