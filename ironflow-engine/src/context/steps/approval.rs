//! Human approval gate for [`WorkflowContext`].

use chrono::{TimeDelta, Utc};
use serde_json::{json, to_value};
use tracing::{info, warn};

use ironflow_store::models::{NewStep, Step, StepKind, StepStatus, StepUpdate, step_trace_id};

use crate::config::{ApprovalConfig, Approvers};
use crate::context::WorkflowContext;
use crate::context::lifecycle::check_replay_identity;
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
    /// On resume, the approval step is replayed: it is marked `Completed`
    /// only when it carries the votes it needs (one, or `required_approvers`
    /// of its [`ApprovalRequirement`](crate::config::ApprovalRequirement)),
    /// and execution continues past it. A gate replayed without them
    /// suspends the run again with [`EngineError::ApprovalRequired`]. Multiple
    /// approval gates in the same handler work -- each one pauses and resumes
    /// independently.
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
    /// When the config carries [`Approvers`]
    /// ([`ApprovalConfig::requiring`]),
    /// they are recorded on the step as an
    /// [`ApprovalRequirement`](crate::config::ApprovalRequirement) when the gate
    /// opens. That record stays the source of truth on replay and resume: the
    /// approvers the handler computes on a later execution are ignored. A config
    /// without approvers stores no requirement.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::ApprovalRequired`] to pause the run on
    /// first execution, and on a replay of a gate that does not carry the
    /// votes it needs yet. Returns [`EngineError::ApprovalRejected`] when an
    /// interceptor refuses the gate. Returns [`EngineError::ReplayDivergence`]
    /// when the step recorded at this position has a different name or kind.
    /// Returns other [`EngineError`] variants on store failures.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::context::WorkflowContext;
    /// use ironflow_engine::config::{ApprovalConfig, Approvers};
    /// use ironflow_engine::error::EngineError;
    /// use serde::Deserialize;
    ///
    /// #[derive(Deserialize)]
    /// struct Payment {
    ///     amount: u64,
    /// }
    ///
    /// # async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    /// let payment: Payment = ctx.input().await?;
    /// let approvers = match payment.amount {
    ///     a if a > 10_000 => Approvers::at_least(2).from_groups(["finance"]).because("amount > 10k"),
    ///     _ => Approvers::any(),
    /// };
    /// ctx.approval("payment-gate", ApprovalConfig::new("Release the payment?").requiring(approvers))
    ///     .await?;
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
            recorder.record_or_stop(name, StepKind::Approval, &self.workflow_name, None)?;
            recorder.set_last(vec![name.to_string()]);
            return Ok(());
        }

        let position = self.position;
        self.position += 1;

        // Replay: if this approval step exists from a prior execution, it is
        // completed only once it carries the votes it needs; a gate replayed
        // without them (a run moved back to `Running` outside the approve
        // route) suspends the run again instead of approving itself.
        if let Some(existing) = self.replay_steps.get(&position) {
            check_replay_identity(existing, position, name, &StepKind::Approval)?;

            if existing.status.state == StepStatus::AwaitingApproval {
                if !gate_has_its_votes(existing) {
                    self.last_step_ids = vec![existing.id];
                    warn!(
                        run_id = %self.run_id,
                        step = %name,
                        position,
                        approvals = existing.approvals.len(),
                        required = required_votes(existing),
                        "approval gate replayed without its votes, run suspended again"
                    );
                    return Err(EngineError::ApprovalRequired {
                        run_id: self.run_id,
                        step_id: existing.id,
                        message: config.message().to_string(),
                    });
                }
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

        // The approvers are recorded only when the gate opens, never on replay
        // or carry-over: the stored requirement is the source of truth from here.
        let requirement = config.approvers().map(Approvers::to_requirement);
        if let Some(requirement) = &requirement {
            info!(
                run_id = %self.run_id,
                step = %name,
                reason = ?requirement.reason,
                required_approvers = requirement.required_approvers,
                approver_groups = ?requirement.approver_groups,
                "approval requirement recorded"
            );
        }

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

/// Votes an approval gate needs: `required_approvers` of its requirement, or
/// one for a gate opened without approvers.
pub(crate) fn required_votes(step: &Step) -> u32 {
    step.approval_requirement
        .as_ref()
        .map_or(1, |requirement| requirement.required_approvers)
}

/// Whether the votes recorded on an approval gate resolve it, counted like the
/// approve route does.
fn gate_has_its_votes(step: &Step) -> bool {
    step.approvals.len() as u32 >= required_votes(step)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use chrono::Utc;
    use serde_json::json;
    use uuid::Uuid;

    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::models::{
        ApprovalRequirement, NewRun, NewStep, Step, StepApproval, StepKind, StepUpdate,
        TriggerKind, step_trace_id,
    };
    use ironflow_store::store::RunStore;

    use super::gate_has_its_votes;

    async fn gate(requirement: Option<u32>, votes: usize) -> Step {
        let store = InMemoryStore::new();
        let run = store
            .create_run(NewRun {
                workflow_name: "gated".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 0,
                handler_version: None,
                labels: HashMap::new(),
                scheduled_at: None,
                created_by: None,
                idempotency_key: None,
                concurrency_key: None,
                priority: 0,
                concurrency_limits: Vec::new(),
                max_cost_usd: None,
                worker_tags: Vec::new(),
            })
            .await
            .expect("create run")
            .into_run();
        let step = store
            .create_step(NewStep {
                run_id: run.id,
                trace_id: step_trace_id(run.id, "gate", 0),
                name: "gate".to_string(),
                kind: StepKind::Approval,
                position: 0,
                input: None,
                is_error_handler: false,
            })
            .await
            .expect("create step");
        store
            .update_step(
                step.id,
                StepUpdate {
                    approval_requirement: requirement.map(|required_approvers| {
                        ApprovalRequirement {
                            required_approvers,
                            ..ApprovalRequirement::default()
                        }
                    }),
                    ..StepUpdate::default()
                },
            )
            .await
            .expect("record the requirement");
        for i in 0..votes {
            store
                .record_step_approval(
                    step.id,
                    StepApproval {
                        user_id: Uuid::now_v7(),
                        approved_by: format!("approver-{i}"),
                        at: Utc::now(),
                    },
                )
                .await
                .expect("record vote");
        }
        store
            .get_step(step.id)
            .await
            .expect("get step")
            .expect("step exists")
    }

    #[tokio::test]
    async fn gate_without_requirement_needs_one_vote() {
        assert!(!gate_has_its_votes(&gate(None, 0).await));
        assert!(gate_has_its_votes(&gate(None, 1).await));
    }

    #[tokio::test]
    async fn gate_with_requirement_needs_its_required_approvers() {
        assert!(!gate_has_its_votes(&gate(Some(2), 0).await));
        assert!(!gate_has_its_votes(&gate(Some(2), 1).await));
        assert!(gate_has_its_votes(&gate(Some(2), 2).await));
        assert!(gate_has_its_votes(&gate(Some(2), 3).await));
    }
}
