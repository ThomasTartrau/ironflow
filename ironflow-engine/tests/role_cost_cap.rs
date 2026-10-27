//! Integration tests for the cost cap of agent roles.
//!
//! An [`AgentRole`] with a cost cap is bounded over a calendar period
//! across every run of every workflow. The engine is real (`Engine::new` with
//! an in-memory store); the agent provider is the harness mock playing the
//! model, reporting a fixed cost per call.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use chrono::{TimeZone, Utc};
use rust_decimal::Decimal;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

use ironflow_core::error::AgentError;
use ironflow_core::provider::{AgentConfig, AgentOutput, AgentProvider, InvokeFuture};
use ironflow_core::retry::RetryPolicy;
use ironflow_engine::config::{AgentRole, AgentStep, RoleCostCap, StepConfig};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::Engine;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_engine::testing::MockAgentProvider;
use ironflow_store::agent_role_store::AgentRoleStore;
use ironflow_store::entities::{RoleCostReservationRequest, RoleCostSettlement};
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{Run, RunFilter, RunStatus, Step, StepStatus, TriggerKind};
use ironflow_store::store::RunStore;

/// Test timeout for bodies that touch the store.
const TEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Cost the mock model reports for every call, in USD.
const STEP_COST: f64 = 0.6;

#[derive(Debug, Deserialize, JsonSchema)]
struct Verdict {
    #[allow(dead_code)]
    approved: bool,
}

/// The role under test: `reviewer`, capped at 1 USD a month.
struct Reviewer;

impl AgentRole for Reviewer {
    type Input = String;
    type Output = Verdict;

    fn prompt(&self, input: String) -> String {
        format!("Review: {input}")
    }

    fn name(&self) -> &str {
        "reviewer"
    }

    fn cost_cap(&self) -> Option<RoleCostCap> {
        Some(RoleCostCap::per_month(Decimal::ONE))
    }
}

/// Named, without a code cap: its cost is only counted.
struct Triage;

impl AgentRole for Triage {
    type Input = String;
    type Output = Verdict;

    fn prompt(&self, input: String) -> String {
        format!("Triage: {input}")
    }

    fn name(&self) -> &str {
        "triage"
    }
}

/// Keeps its default name, its type name: counted as `Anonymous`.
struct Anonymous;

impl AgentRole for Anonymous {
    type Input = String;
    type Output = Verdict;

    fn prompt(&self, input: String) -> String {
        format!("Anything: {input}")
    }
}

/// Runs `n` sequential tasks of [`Reviewer`].
struct Reviews {
    name: &'static str,
    n: usize,
}

impl WorkflowHandler for Reviews {
    fn name(&self) -> &str {
        self.name
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            for i in 0..self.n {
                ctx.agent(&format!("review-{i}"), Reviewer.task(format!("diff {i}")))
                    .await?;
            }
            Ok(())
        })
    }
}

/// One task of [`Triage`] and one of [`Anonymous`].
struct Untracked;

impl WorkflowHandler for Untracked {
    fn name(&self) -> &str {
        "untracked"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.agent("triage", Triage.task("issue".to_string()))
                .await?;
            ctx.agent("anything", Anonymous.task("issue".to_string()))
                .await?;
            Ok(())
        })
    }
}

/// Two [`Reviewer`] tasks budgeted at 1 USD each, in one parallel wave.
struct ParallelReviews;

impl WorkflowHandler for ParallelReviews {
    fn name(&self) -> &str {
        "parallel-reviews"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let steps: Vec<(&str, StepConfig)> = ["left", "right"]
                .into_iter()
                .map(|name| {
                    let task = Reviewer.task(name.to_string()).max_budget_usd(1.0);
                    (name, StepConfig::Agent(task.into_config()))
                })
                .collect();
            ctx.parallel(steps, false).await?;
            Ok(())
        })
    }
}

/// One [`Reviewer`] task with a retry policy whose backoff outlasts the test.
struct RetriedReview;

impl WorkflowHandler for RetriedReview {
    fn name(&self) -> &str {
        "retried-review"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let retry = RetryPolicy::new(2)
                .backoff(Duration::from_secs(60))
                .max_backoff(Duration::from_secs(60));
            ctx.agent(
                "review",
                Reviewer.task("diff".to_string()).retry_policy(retry),
            )
            .await?;
            Ok(())
        })
    }
}

/// The mock model, counting its calls.
fn model(calls: Arc<AtomicUsize>) -> Arc<dyn AgentProvider> {
    Arc::new(MockAgentProvider::new(move |_| {
        calls.fetch_add(1, Ordering::SeqCst);
        let mut output = AgentOutput::new(json!({"approved": true}));
        output.cost_usd = Some(STEP_COST);
        Ok(output)
    }))
}

/// A model that takes a while to answer, so parallel steps overlap.
struct SlowModel {
    calls: Arc<AtomicUsize>,
}

impl AgentProvider for SlowModel {
    fn invoke<'a>(&'a self, _config: &'a AgentConfig) -> InvokeFuture<'a> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            sleep(Duration::from_millis(200)).await;
            let mut output = AgentOutput::new(json!({"approved": true}));
            output.cost_usd = Some(STEP_COST);
            Ok::<AgentOutput, AgentError>(output)
        })
    }
}

fn engine(store: &Arc<InMemoryStore>, provider: Arc<dyn AgentProvider>) -> Engine {
    let mut engine = Engine::new(store.clone(), provider);
    engine
        .register(Reviews {
            name: "review-a",
            n: 1,
        })
        .expect("register");
    engine
        .register(Reviews {
            name: "review-b",
            n: 1,
        })
        .expect("register");
    engine
        .register(Reviews {
            name: "three-reviews",
            n: 3,
        })
        .expect("register");
    engine.register(Untracked).expect("register");
    engine.register(ParallelReviews).expect("register");
    engine.register(RetriedReview).expect("register");
    engine
}

/// Run `workflow` and return its run, whether it completed or failed.
async fn run(engine: &Engine, store: &InMemoryStore, workflow: &str) -> Run {
    let outcome = engine
        .run_handler(workflow, TriggerKind::Api, json!({}))
        .await;
    if let Ok(result) = outcome {
        return result.run;
    }
    let runs = store
        .list_runs(
            RunFilter {
                workflow_name: Some(workflow.to_string()),
                ..RunFilter::default()
            },
            1,
            100,
        )
        .await
        .expect("list runs");
    runs.items
        .into_iter()
        .max_by_key(|run| run.id)
        .expect("the run was created")
}

async fn steps(store: &InMemoryStore, run: &Run) -> Vec<Step> {
    store.list_steps(run.id).await.expect("list steps")
}

async fn spent(store: &InMemoryStore, role: &str) -> Decimal {
    store
        .get_agent_role(role)
        .await
        .expect("get role")
        .expect("role is tracked")
        .spent_usd
}

#[tokio::test]
async fn capped_role_refuses_the_step_once_its_monthly_cap_is_reached() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let calls = Arc::new(AtomicUsize::new(0));
        let engine = engine(&store, model(calls.clone()));

        let run = run(&engine, &store, "three-reviews").await;
        assert_eq!(run.status.state, RunStatus::Failed);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "the third step never reached the model"
        );

        let steps = steps(&store, &run).await;
        let refused = steps
            .iter()
            .find(|s| s.name == "review-2")
            .expect("third step recorded");
        assert_eq!(refused.status.state, StepStatus::Failed);
        let error = refused.error.as_deref().expect("step error");
        assert!(error.contains("reviewer"), "{error}");
        assert!(error.contains("cost cap of $1 per month"), "{error}");
        assert_eq!(spent(&store, "reviewer").await, Decimal::new(12, 1));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn the_cap_is_shared_by_runs_of_different_workflows() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let calls = Arc::new(AtomicUsize::new(0));
        let engine = engine(&store, model(calls.clone()));

        assert_eq!(
            run(&engine, &store, "review-a").await.status.state,
            RunStatus::Completed
        );
        assert_eq!(
            run(&engine, &store, "review-b").await.status.state,
            RunStatus::Completed
        );
        let third = run(&engine, &store, "review-a").await;
        assert_eq!(third.status.state, RunStatus::Failed);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn a_role_without_cap_runs_and_its_cost_is_counted() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let calls = Arc::new(AtomicUsize::new(0));
        let engine = engine(&store, model(calls.clone()));

        for _ in 0..3 {
            let run = run(&engine, &store, "untracked").await;
            assert_eq!(run.status.state, RunStatus::Completed);
        }
        assert_eq!(calls.load(Ordering::SeqCst), 6);
        let triage = store
            .get_agent_role("triage")
            .await
            .expect("get role")
            .expect("every role is tracked");
        assert_eq!(triage.spent_usd, Decimal::new(18, 1));
        assert_eq!(triage.effective_cap_usd(), None);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn a_role_with_the_default_name_is_tracked_under_its_type_name() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let calls = Arc::new(AtomicUsize::new(0));
        let engine = engine(&store, model(calls.clone()));

        run(&engine, &store, "untracked").await;
        let roles = store.list_agent_roles(1, 100).await.expect("list roles");
        let names: Vec<_> = roles.items.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["Anonymous", "triage"], "every role is tracked");
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn operator_cap_lowers_the_code_cap() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let calls = Arc::new(AtomicUsize::new(0));
        let engine = engine(&store, model(calls.clone()));

        assert_eq!(
            run(&engine, &store, "review-a").await.status.state,
            RunStatus::Completed
        );
        store
            .set_agent_role_operator_cap("reviewer", Some(Decimal::new(5, 1)))
            .await
            .expect("set operator cap");

        let refused = run(&engine, &store, "review-b").await;
        assert_eq!(refused.status.state, RunStatus::Failed);
        let error = steps(&store, &refused).await[0]
            .error
            .clone()
            .expect("step error");
        assert!(error.contains("cost cap of $0.5 per month"), "{error}");
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        // An operator cap above the code cap does not raise it.
        store
            .set_agent_role_operator_cap("reviewer", Some(Decimal::TEN))
            .await
            .expect("raise operator cap");
        assert_eq!(
            run(&engine, &store, "review-a").await.status.state,
            RunStatus::Completed
        );
        let refused = run(&engine, &store, "review-a").await;
        assert_eq!(refused.status.state, RunStatus::Failed);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn a_parallel_wave_cannot_exceed_the_cap_by_more_than_one_step() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let calls = Arc::new(AtomicUsize::new(0));
        let engine = engine(
            &store,
            Arc::new(SlowModel {
                calls: calls.clone(),
            }),
        );

        let run = run(&engine, &store, "parallel-reviews").await;
        assert_eq!(run.status.state, RunStatus::Failed);
        assert_eq!(calls.load(Ordering::SeqCst), 1, "one step was admitted");
        let steps = steps(&store, &run).await;
        let completed = steps
            .iter()
            .filter(|s| s.status.state == StepStatus::Completed)
            .count();
        let failed: Vec<_> = steps
            .iter()
            .filter(|s| s.status.state == StepStatus::Failed)
            .collect();
        assert_eq!(completed, 1);
        assert_eq!(failed.len(), 1);
        assert!(
            failed[0]
                .error
                .as_deref()
                .is_some_and(|e| e.contains("reviewer"))
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn the_counter_restarts_in_a_new_period() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let calls = Arc::new(AtomicUsize::new(0));
        let engine = engine(&store, model(calls.clone()));

        // Spend 5 USD in a month long gone.
        let old_month = Utc
            .with_ymd_and_hms(2020, 1, 1, 0, 0, 0)
            .single()
            .expect("valid date");
        let reservation_id = Uuid::now_v7();
        store
            .reserve_agent_role_cost(RoleCostReservationRequest {
                reservation_id,
                role: "reviewer".to_string(),
                code_cap_usd: None,
                period: "month".to_string(),
                period_start: old_month,
                budget_usd: None,
                ttl_secs: 600,
            })
            .await
            .expect("seed reservation");
        store
            .settle_agent_role_cost(RoleCostSettlement {
                reservation_id,
                period: "month".to_string(),
                period_start: old_month,
                cost_usd: Decimal::from(5),
            })
            .await
            .expect("seed spend");
        assert_eq!(spent(&store, "reviewer").await, Decimal::from(5));

        let run = run(&engine, &store, "review-a").await;
        assert_eq!(run.status.state, RunStatus::Completed);
        assert_eq!(spent(&store, "reviewer").await, Decimal::new(6, 1));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn a_refused_step_is_not_retried() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let calls = Arc::new(AtomicUsize::new(0));
        let engine = engine(&store, model(calls.clone()));

        // Use up the cap: 0.6 + 0.6 >= 1.
        run(&engine, &store, "review-a").await;
        run(&engine, &store, "review-b").await;
        assert_eq!(calls.load(Ordering::SeqCst), 2);

        // A retry would sleep 60 seconds and blow the test timeout.
        let run = run(&engine, &store, "retried-review").await;
        assert_eq!(run.status.state, RunStatus::Failed);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    })
    .await
    .expect("test timed out");
}
