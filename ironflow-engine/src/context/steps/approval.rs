//! Human approval gate for [`WorkflowContext`].

use chrono::{TimeDelta, Utc};
use serde_json::{json, to_value};
use tracing::info;

use ironflow_store::models::{NewStep, StepKind, StepStatus, StepUpdate, step_trace_id};

use crate::config::ApprovalConfig;
use crate::context::WorkflowContext;
use crate::error::EngineError;
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
    /// # Errors
    ///
    /// Returns [`EngineError::ApprovalRequired`] to pause the run on
    /// first execution. Returns other [`EngineError`] variants on store
    /// failures.
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
