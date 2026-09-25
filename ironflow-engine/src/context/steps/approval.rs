//! Human approval gate for [`WorkflowContext`].

use chrono::{TimeDelta, Utc};
use serde_json::{Map, Value, json, to_value};
use tracing::info;
use uuid::Uuid;

use ironflow_store::error::StoreError;
use ironflow_store::models::{NewStep, Run, Step, StepKind, StepStatus, StepUpdate, step_trace_id};

use crate::config::ApprovalConfig;
use crate::context::WorkflowContext;
use crate::error::EngineError;
use crate::executor::ApprovalOutcome;
use crate::notify::{WorkflowApprovalRequiredEvent, WorkflowEvent};
use crate::plan::lock_plan;

impl WorkflowContext {
    /// Create a human approval gate.
    ///
    /// On first execution, records an approval step and returns
    /// [`EngineError::ApprovalRequired`] to suspend the run. The engine
    /// transitions the run to `AwaitingApproval`.
    ///
    /// On resume (after a human approved via the API), the approval step
    /// is replayed: it is marked as `Completed` and execution continues
    /// past it. Multiple approval gates in the same handler work -- each
    /// one pauses and resumes independently.
    ///
    /// When the config carries an SLA
    /// ([`ApprovalConfig::with_deadline`](crate::config::ApprovalConfig::with_deadline),
    /// or the legacy `with_timeout_seconds`), the deadline is persisted on the
    /// step so the API server's escalator can apply the configured
    /// [`EscalationPolicy`](crate::config::EscalationPolicy) when it fires. The
    /// timer is cleared as soon as the gate resolves.
    ///
    /// A [`StepInterceptor`](crate::executor::StepInterceptor) wired into the
    /// context resolves the gate inline instead of suspending: the step is
    /// recorded, then completed or rejected without waiting for a human. This
    /// is what [`crate::testing::TestEngine`] uses to run gated handlers end to
    /// end.
    ///
    /// When the config carries approval rules
    /// ([`ApprovalConfig::with_rule`](crate::config::ApprovalConfig::with_rule)),
    /// they are evaluated once, when the gate opens, against a context holding
    /// the `output` of the previous step (the last one of a parallel batch),
    /// the run `payload` and `labels`, run `metadata` (`run_id`,
    /// `workflow_name`, `trigger`, `attempt`, `handler_version`) and the
    /// completed steps of the current attempt under `steps.<name>` (`output`,
    /// `kind`, `status`). The resulting
    /// [`ApprovalRequirement`](crate::config::ApprovalRequirement) is stored on
    /// the step and stays the source of truth on replay and resume: rules are
    /// never re-evaluated. A config without rules stores no requirement.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::ApprovalRequired`] to pause the run on
    /// first execution. Returns [`EngineError::ApprovalRejected`] when an
    /// interceptor refuses the gate. Returns other [`EngineError`] variants on
    /// store failures.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::context::WorkflowContext;
    /// use ironflow_engine::config::ApprovalConfig;
    /// use ironflow_engine::error::EngineError;
    ///
    /// # async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    /// ctx.approval("deploy-gate", ApprovalConfig::new("Approve deployment?")).await?;
    /// // Execution continues here after approval
    /// # Ok(())
    /// # }
    /// ```
    pub async fn approval(
        &mut self,
        name: &str,
        config: ApprovalConfig,
    ) -> Result<(), EngineError> {
        // Plan mode: record the gate and continue. Planning must never suspend,
        // so this comes before the `ApprovalRequired` path below.
        if let Some(plan) = self.plan().cloned() {
            self.position += 1;
            let mut recorder = lock_plan(&plan);
            if recorder.record(name, StepKind::Approval, &self.workflow_name, None) {
                recorder.set_last(vec![name.to_string()]);
            }
            return Ok(());
        }

        let position = self.position;
        self.position += 1;

        // Replay: if this approval step exists from a prior execution,
        // the run was approved -- mark it completed (if not already) and continue.
        if let Some(existing) = self.replay_steps.get(&position)
            && existing.kind == StepKind::Approval
        {
            if existing.status.state == StepStatus::AwaitingApproval {
                self.store
                    .update_step(
                        existing.id,
                        StepUpdate {
                            status: Some(StepStatus::Completed),
                            completed_at: Some(Utc::now()),
                            // An approved gate must never be escalated afterwards.
                            clear_approval_deadline: true,
                            ..StepUpdate::default()
                        },
                    )
                    .await?;
            }

            self.last_step_ids = vec![existing.id];
            info!(
                run_id = %self.run_id,
                step = %name,
                position,
                "approval step replayed (approved)"
            );
            return Ok(());
        }

        // Carried over: a human already approved this gate in an earlier
        // attempt. Record a fresh step in the current attempt so that each
        // attempt keeps a complete, self-contained DAG, and continue.
        if let Some(&granted_in) = self.granted_approvals.get(&position) {
            let trace_id = step_trace_id(self.run_id, name, position);
            let step = self
                .store
                .create_step(NewStep {
                    run_id: self.run_id,
                    trace_id,
                    name: name.to_string(),
                    kind: StepKind::Approval,
                    position,
                    input: Some(to_value(&config)?),
                    is_error_handler: false,
                })
                .await?;

            let now = Utc::now();
            self.start_step(step.id, now).await?;
            self.store
                .update_step(
                    step.id,
                    StepUpdate {
                        status: Some(StepStatus::Completed),
                        output: Some(json!({"approved_in_attempt": granted_in})),
                        completed_at: Some(now),
                        ..StepUpdate::default()
                    },
                )
                .await?;

            self.last_step_ids = vec![step.id];
            info!(
                run_id = %self.run_id,
                step = %name,
                position,
                granted_in_attempt = granted_in,
                attempt = self.attempt,
                "approval carried over from a previous attempt"
            );
            return Ok(());
        }

        // Rules are evaluated only when the gate opens, never on replay or
        // carry-over: the stored requirement is the source of truth from here.
        let requirement = if config.rules().is_empty() {
            None
        } else {
            let run = self
                .store
                .get_run(self.run_id)
                .await?
                .ok_or(EngineError::Store(StoreError::RunNotFound(self.run_id)))?;
            let steps = self.store.list_steps(self.run_id).await?;
            let ctx = approval_expression_context(
                &run,
                &steps,
                self.attempt,
                position,
                self.last_step_ids.last().copied(),
            );
            let requirement = config.evaluate_rules(&ctx);
            info!(
                run_id = %self.run_id,
                step = %name,
                rule_index = ?requirement.rule_index,
                required_approvers = requirement.required_approvers,
                approver_groups = ?requirement.approver_groups,
                "approval rules evaluated"
            );
            Some(requirement)
        };

        // An interceptor resolves the gate inline: the run neither suspends nor
        // waits for a human.
        if let Some(interceptor) = self.interceptor.clone()
            && let Some(outcome) = interceptor.intercept_approval(name, &config)
        {
            let trace_id = step_trace_id(self.run_id, name, position);
            let step = self
                .store
                .create_step(NewStep {
                    run_id: self.run_id,
                    trace_id,
                    name: name.to_string(),
                    kind: StepKind::Approval,
                    position,
                    input: Some(to_value(&config)?),
                    is_error_handler: false,
                })
                .await?;

            let now = Utc::now();
            self.start_step(step.id, now).await?;
            self.last_step_ids = vec![step.id];

            return match outcome {
                ApprovalOutcome::Approved => {
                    self.store
                        .update_step(
                            step.id,
                            StepUpdate {
                                status: Some(StepStatus::Completed),
                                output: Some(json!({"approved_by": "step-interceptor"})),
                                completed_at: Some(now),
                                approval_requirement: requirement.clone(),
                                ..StepUpdate::default()
                            },
                        )
                        .await?;
                    info!(
                        run_id = %self.run_id,
                        step = %name,
                        position,
                        "approval granted by the step interceptor"
                    );
                    Ok(())
                }
                ApprovalOutcome::Rejected { reason } => {
                    // The step FSM only reaches Rejected from AwaitingApproval.
                    self.store
                        .update_step(
                            step.id,
                            StepUpdate {
                                status: Some(StepStatus::AwaitingApproval),
                                approval_requirement: requirement.clone(),
                                ..StepUpdate::default()
                            },
                        )
                        .await?;
                    self.store
                        .update_step(
                            step.id,
                            StepUpdate {
                                status: Some(StepStatus::Rejected),
                                error: Some(reason.clone()),
                                completed_at: Some(Utc::now()),
                                ..StepUpdate::default()
                            },
                        )
                        .await?;
                    info!(
                        run_id = %self.run_id,
                        step = %name,
                        position,
                        %reason,
                        "approval rejected by the step interceptor"
                    );
                    Err(EngineError::ApprovalRejected {
                        run_id: self.run_id,
                        step_id: step.id,
                        reason,
                    })
                }
            };
        }

        // First execution: create the approval step and suspend.
        let trace_id = step_trace_id(self.run_id, name, position);
        let step = self
            .store
            .create_step(NewStep {
                run_id: self.run_id,
                trace_id,
                name: name.to_string(),
                kind: StepKind::Approval,
                position,
                input: Some(to_value(&config)?),
                is_error_handler: false,
            })
            .await?;

        self.start_step(step.id, Utc::now()).await?;

        // Transition the step to AwaitingApproval so it reflects the suspended
        // state on the dashboard, and arm the SLA timer in the same update. The
        // deadline lives in the store, so it survives an API or worker restart.
        let deadline_at = config
            .effective_deadline_secs()
            .map(|secs| Utc::now() + TimeDelta::seconds(secs as i64));

        self.store
            .update_step(
                step.id,
                StepUpdate {
                    status: Some(StepStatus::AwaitingApproval),
                    approval_deadline_at: deadline_at,
                    approval_stage: Some(0),
                    approval_assignee: config.assignee().cloned(),
                    approval_requirement: requirement,
                    ..StepUpdate::default()
                },
            )
            .await?;

        self.last_step_ids = vec![step.id];

        if let Some(ref bus) = self.event_bus {
            bus.publish(
                self.run_id,
                WorkflowEvent::ApprovalRequired(WorkflowApprovalRequiredEvent {
                    step_name: name.to_string(),
                    step_index: position,
                    approval_id: step.id,
                }),
            );
        }

        Err(EngineError::ApprovalRequired {
            run_id: self.run_id,
            step_id: step.id,
            message: config.message().to_string(),
        })
    }
}

/// Build the JSON context approval rule conditions are evaluated against.
///
/// - `output`: output of the step `last_step_id` (the previous step, or the
///   last one of a parallel batch), `null` when absent.
/// - `payload`, `labels`: taken from the run.
/// - `metadata`: `run_id`, `workflow_name`, `trigger`, `attempt`,
///   `handler_version`.
/// - `steps.<name>`: `output`, `kind` and `status` of every completed step of
///   `attempt` positioned before `before_position`. When two steps share a
///   name, the one with the higher position wins.
pub(crate) fn approval_expression_context(
    run: &Run,
    steps: &[Step],
    attempt: u32,
    before_position: u32,
    last_step_id: Option<Uuid>,
) -> Value {
    let output = last_step_id
        .and_then(|id| steps.iter().find(|s| s.id == id))
        .and_then(|s| s.output.clone())
        .unwrap_or(Value::Null);

    let mut completed: Vec<&Step> = steps
        .iter()
        .filter(|s| {
            s.attempt == attempt
                && s.position < before_position
                && s.status.state == StepStatus::Completed
        })
        .collect();
    completed.sort_by_key(|s| s.position);

    let mut by_name = Map::new();
    for step in completed {
        by_name.insert(
            step.name.clone(),
            json!({
                "output": step.output,
                "kind": step.kind,
                "status": step.status.state,
            }),
        );
    }

    json!({
        "output": output,
        "payload": run.payload,
        "labels": run.labels,
        "metadata": {
            "run_id": run.id,
            "workflow_name": run.workflow_name,
            "trigger": run.trigger,
            "attempt": attempt,
            "handler_version": run.handler_version,
        },
        "steps": by_name,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::slice;

    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::models::{NewRun, TriggerKind};
    use ironflow_store::store::RunStore;

    use super::*;

    async fn run_with_labels(store: &InMemoryStore) -> Run {
        store
            .create_run(NewRun {
                created_by: None,
                workflow_name: "payments".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({"amount": 15000}),
                max_retries: 0,
                handler_version: Some("v2".to_string()),
                labels: HashMap::from([("env".to_string(), "production".to_string())]),
                scheduled_at: None,
                idempotency_key: None,
                max_cost_usd: None,
            })
            .await
            .expect("create run")
            .into_run()
    }

    /// Create a step at `position`, optionally driving it to `Completed`.
    async fn step(
        store: &InMemoryStore,
        run_id: Uuid,
        name: &str,
        position: u32,
        output: Option<Value>,
    ) -> Step {
        let step = store
            .create_step(NewStep {
                run_id,
                trace_id: step_trace_id(run_id, name, position),
                name: name.to_string(),
                kind: StepKind::Shell,
                position,
                input: None,
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
        if output.is_some() {
            store
                .update_step(
                    step.id,
                    StepUpdate {
                        status: Some(StepStatus::Completed),
                        output,
                        ..StepUpdate::default()
                    },
                )
                .await
                .expect("to completed");
        }
        store.get_step(step.id).await.expect("get").expect("exists")
    }

    #[tokio::test]
    async fn context_holds_run_data_and_the_previous_output() {
        let store = InMemoryStore::new();
        let run = run_with_labels(&store).await;
        let risk = step(&store, run.id, "risk", 0, Some(json!({"level": "high"}))).await;

        let steps = slice::from_ref(&risk);
        let ctx = approval_expression_context(&run, steps, 1, 1, Some(risk.id));

        assert_eq!(ctx["output"], json!({"level": "high"}));
        assert_eq!(ctx["payload"], json!({"amount": 15000}));
        assert_eq!(ctx["labels"], json!({"env": "production"}));
        assert_eq!(ctx["metadata"]["run_id"], json!(run.id));
        assert_eq!(ctx["metadata"]["workflow_name"], json!("payments"));
        assert_eq!(ctx["metadata"]["trigger"], json!(run.trigger));
        assert_eq!(ctx["metadata"]["attempt"], json!(1));
        assert_eq!(ctx["metadata"]["handler_version"], json!("v2"));
        assert_eq!(ctx["steps"]["risk"]["output"], json!({"level": "high"}));
        assert_eq!(ctx["steps"]["risk"]["kind"], json!("shell"));
        assert_eq!(ctx["steps"]["risk"]["status"], json!("completed"));
    }

    #[tokio::test]
    async fn output_is_null_without_a_previous_step() {
        let store = InMemoryStore::new();
        let run = run_with_labels(&store).await;

        let ctx = approval_expression_context(&run, &[], 1, 0, None);

        assert_eq!(ctx["output"], Value::Null);
        assert_eq!(ctx["steps"], json!({}));
    }

    #[tokio::test]
    async fn steps_keep_only_completed_steps_of_the_attempt_before_the_gate() {
        let store = InMemoryStore::new();
        let run = run_with_labels(&store).await;
        let done = step(&store, run.id, "done", 0, Some(json!(1))).await;
        let running = step(&store, run.id, "running", 1, None).await;
        let later = step(&store, run.id, "later", 5, Some(json!(2))).await;
        let mut previous_attempt = step(&store, run.id, "old", 2, Some(json!(3))).await;
        previous_attempt.attempt = 2;

        let steps = [done.clone(), running, later, previous_attempt];
        let ctx = approval_expression_context(&run, &steps, 1, 3, Some(done.id));

        let names: Vec<&String> = ctx["steps"]
            .as_object()
            .expect("steps object")
            .keys()
            .collect();
        assert_eq!(names, vec!["done"]);
        assert_eq!(ctx["output"], json!(1));
    }

    #[tokio::test]
    async fn duplicate_names_keep_the_highest_position() {
        let store = InMemoryStore::new();
        let run = run_with_labels(&store).await;
        let second = step(&store, run.id, "check", 1, Some(json!("second"))).await;
        let first = step(&store, run.id, "check", 0, Some(json!("first"))).await;

        let ctx = approval_expression_context(&run, &[second, first], 1, 2, None);

        assert_eq!(ctx["steps"]["check"]["output"], json!("second"));
    }
}
