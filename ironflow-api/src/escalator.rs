//! Resolution of approval gates that missed their SLA deadline.
//!
//! An approval gate can carry a deadline. The deadline lives on the step row, so
//! it survives an API or worker restart. The [`Escalator`] periodically hands
//! every expired gate to the engine's
//! [`ApprovalEscalator`](ironflow_engine::escalation::ApprovalEscalator), which
//! applies the gate's configured escalation policy.
//!
//! Gates without a deadline are never touched, and a deadline fires at most once
//! even when several API instances run this loop.

use std::sync::Arc;
use std::time::Duration;

use ironflow_engine::engine::Engine;
use ironflow_engine::escalation::{ApprovalEscalator, EscalationAction};
use tokio::time::interval;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

/// How often expired approval deadlines are collected.
pub const DEFAULT_ESCALATOR_INTERVAL: Duration = Duration::from_secs(30);

/// How many gates a single tick resolves.
///
/// Bounded so that a burst of expiries (a long outage with many open gates)
/// resorbs progressively instead of holding a long transaction on the steps
/// table.
pub const DEFAULT_ESCALATOR_BATCH_SIZE: u32 = 50;

/// Periodic task that escalates approval gates past their deadline.
///
/// # Examples
///
/// ```no_run
/// use std::sync::Arc;
/// use std::time::Duration;
/// use ironflow_api::escalator::Escalator;
/// use ironflow_core::providers::claude::ClaudeCodeProvider;
/// use ironflow_engine::engine::Engine;
/// use ironflow_store::memory::InMemoryStore;
/// use ironflow_store::store::Store;
/// use tokio_util::sync::CancellationToken;
///
/// # async fn example() {
/// let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
/// let engine = Arc::new(Engine::new(store, Arc::new(ClaudeCodeProvider::new())));
///
/// let escalator = Escalator::new(engine).interval(Duration::from_secs(15));
/// tokio::spawn(escalator.run(CancellationToken::new()));
/// # }
/// ```
pub struct Escalator {
    escalator: ApprovalEscalator,
    interval: Duration,
}

impl Escalator {
    /// Create an escalator with the default interval and batch size.
    pub fn new(engine: Arc<Engine>) -> Self {
        Self {
            escalator: ApprovalEscalator::new(engine).batch_size(DEFAULT_ESCALATOR_BATCH_SIZE),
            interval: DEFAULT_ESCALATOR_INTERVAL,
        }
    }

    /// Set how often expired deadlines are collected.
    ///
    /// Keep it well below the shortest SLA in use, otherwise a gate overshoots
    /// its deadline by up to one interval.
    pub fn interval(mut self, interval: Duration) -> Self {
        self.interval = interval;
        self
    }

    /// Set how many gates a single tick resolves.
    pub fn batch_size(self, batch_size: u32) -> Self {
        Self {
            escalator: self.escalator.batch_size(batch_size),
            interval: self.interval,
        }
    }

    /// Run the escalation loop until `shutdown` is cancelled.
    ///
    /// Store errors are logged and the loop keeps going: a transient database
    /// failure must not silently stop escalation.
    pub async fn run(self, shutdown: CancellationToken) {
        let mut ticker = interval(self.interval);
        // The first tick fires immediately; skip it so startup is not a burst.
        ticker.tick().await;

        info!(
            interval_secs = self.interval.as_secs(),
            "approval escalator started"
        );

        loop {
            tokio::select! {
                _ = shutdown.cancelled() => {
                    info!("approval escalator stopped");
                    return;
                }
                _ = ticker.tick() => {
                    self.tick().await;
                }
            }
        }
    }

    /// Escalate one batch of expired gates.
    ///
    /// Exposed for tests and for callers that drive the schedule themselves.
    pub async fn tick(&self) {
        let records = match self.escalator.tick().await {
            Ok(records) => records,
            Err(err) => {
                error!(error = %err, "failed to collect expired approval deadlines");
                return;
            }
        };

        for record in &records {
            // A stale gate resolved on its own between the claim and the
            // escalation; that is routine, not news.
            if record.action == EscalationAction::Stale {
                continue;
            }

            info!(
                run_id = %record.run_id,
                step_id = %record.step_id,
                stage = record.stage,
                action = ?record.action,
                reason = %record.reason,
                "approval gate escalated"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeDelta, Utc};
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::config::{ApprovalConfig, EscalationPolicy};
    use ironflow_store::entities::{
        NewRun, NewStep, RunStatus, StepKind, StepStatus, StepUpdate, TriggerKind, step_trace_id,
    };
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::store::{RunStore, Store};
    use serde_json::json;
    use std::collections::HashMap;
    use uuid::Uuid;

    use super::*;

    /// Build an escalator over a store holding one run stuck on an expired gate.
    async fn expired_gate(config: ApprovalConfig) -> (Arc<InMemoryStore>, Escalator, Uuid, Uuid) {
        let store = Arc::new(InMemoryStore::new());
        let store_dyn: Arc<dyn Store> = store.clone();
        let engine = Arc::new(Engine::new(store_dyn, Arc::new(ClaudeCodeProvider::new())));

        let run = store
            .create_run(NewRun {
                created_by: None,
                workflow_name: "deploy".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 0,
                handler_version: None,
                labels: HashMap::new(),
                scheduled_at: None,
                idempotency_key: None,
                max_cost_usd: None,
            })
            .await
            .expect("create run")
            .into_run();
        store
            .update_run_status(run.id, RunStatus::Running)
            .await
            .expect("to running");
        store
            .update_run_status(run.id, RunStatus::AwaitingApproval)
            .await
            .expect("to awaiting approval");

        let step = store
            .create_step(NewStep {
                run_id: run.id,
                trace_id: step_trace_id(run.id, "prod-gate", 0),
                name: "prod-gate".to_string(),
                kind: StepKind::Approval,
                position: 0,
                input: Some(serde_json::to_value(&config).expect("serialize config")),
                is_error_handler: false,
            })
            .await
            .expect("create step");
        store
            .update_step(
                step.id,
                StepUpdate {
                    status: Some(StepStatus::Running),
                    ..StepUpdate::default()
                },
            )
            .await
            .expect("to running");
        store
            .update_step(
                step.id,
                StepUpdate {
                    status: Some(StepStatus::AwaitingApproval),
                    approval_deadline_at: Some(Utc::now() - TimeDelta::seconds(1)),
                    ..StepUpdate::default()
                },
            )
            .await
            .expect("arm an expired timer");

        (store, Escalator::new(engine), run.id, step.id)
    }

    #[tokio::test]
    async fn tick_auto_rejects_a_gate_past_its_deadline() {
        let config = ApprovalConfig::new("Deploy?")
            .with_deadline_secs(60)
            .on_timeout(EscalationPolicy::AutoReject);
        let (store, escalator, run_id, step_id) = expired_gate(config).await;

        escalator.tick().await;

        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::Failed);
        assert_eq!(run.error.as_deref(), Some("approval timeout"));

        let step = store.get_step(step_id).await.unwrap().unwrap();
        assert_eq!(step.status.state, StepStatus::Failed);
        assert_eq!(step.error.as_deref(), Some("approval timeout"));
        assert!(step.approval_deadline_at.is_none());
    }

    #[tokio::test]
    async fn tick_leaves_a_gate_without_a_deadline_alone() {
        let config = ApprovalConfig::new("Deploy?");
        let (store, escalator, run_id, step_id) = expired_gate(config).await;

        // Drop the timer the fixture armed: this gate carries no SLA.
        store
            .update_step(
                step_id,
                StepUpdate {
                    clear_approval_deadline: true,
                    ..StepUpdate::default()
                },
            )
            .await
            .expect("clear timer");

        escalator.tick().await;

        let run = store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunStatus::AwaitingApproval);
        let step = store.get_step(step_id).await.unwrap().unwrap();
        assert_eq!(step.status.state, StepStatus::AwaitingApproval);
    }

    #[tokio::test]
    async fn run_stops_on_shutdown() {
        let config = ApprovalConfig::new("Deploy?");
        let (_store, escalator, _run_id, _step_id) = expired_gate(config).await;
        let shutdown = CancellationToken::new();
        shutdown.cancel();

        // Returns instead of looping forever.
        tokio::time::timeout(Duration::from_secs(5), escalator.run(shutdown))
            .await
            .expect("escalator stopped");
    }
}
