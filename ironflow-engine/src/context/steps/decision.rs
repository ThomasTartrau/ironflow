//! Typed machine-decision step for [`WorkflowContext`].
//!
//! Holds the public [`decision`](WorkflowContext::decision) entry point and the
//! replay, execute and escalate paths it dispatches to. As a descendant module
//! of `context`, it can access `WorkflowContext`'s private fields.
//!
//! The decision step is a hybrid of an agent step (it calls a provider, costs
//! money, and stores typed output) and an approval gate (a low-confidence answer
//! suspends the run and replays its stored answers on resume).

use std::collections::BTreeMap;

use chrono::Utc;
use serde_json::{Value, from_value, to_value};
use tracing::info;
use uuid::Uuid;

use ironflow_core::decision::{DecisionOutput, DecisionUsage};
use ironflow_store::models::{NewStep, StepKind, StepStatus, StepUpdate, step_trace_id};

use crate::config::DecisionConfig;
use crate::context::WorkflowContext;
use crate::error::EngineError;
use crate::executor::{DecisionExecution, StepOutput, StepResult, execute_decision};
use crate::notify::{
    WorkflowApprovalRequiredEvent, WorkflowEvent, WorkflowStepCompletedEvent,
    WorkflowStepStartedEvent,
};
use crate::plan::lock_plan;

impl WorkflowContext {
    /// Execute a typed machine-decision step (System One / Jev).
    ///
    /// See [`DecisionConfig`]. Returns a
    /// [`DecisionOutput`] whose answers are accessed by name. When
    /// `escalate_below` is set and any answer falls below it, the run suspends
    /// with [`EngineError::ApprovalRequired`] and replays the stored answers on
    /// resume without re-calling the provider.
    ///
    /// # Errors
    ///
    /// [`EngineError::NoDecisionProvider`], [`EngineError::ApprovalRequired`], or
    /// [`EngineError::Operation`].
    pub async fn decision(
        &mut self,
        name: &str,
        config: DecisionConfig,
    ) -> Result<DecisionOutput, EngineError> {
        // Plan mode: record the step and return an empty answer set. No
        // provider is called, so the step costs nothing while planning.
        if let Some(plan) = self.plan().cloned() {
            self.position += 1;
            let mut recorder = lock_plan(&plan);
            if recorder.record(name, StepKind::Decision, &self.workflow_name, None) {
                recorder.set_last(vec![name.to_string()]);
            }
            return Ok(DecisionOutput {
                model: None,
                answers: BTreeMap::new(),
                usage: DecisionUsage::default(),
            });
        }

        if let Some(output) = self.decision_replay(name, &config).await? {
            return Ok(output);
        }
        self.decision_execute(name, config).await
    }

    /// Replay a decision step recorded in this attempt, if any.
    ///
    /// Returns `Ok(Some(output))` when the step at the current position was
    /// already decided (its answers are returned as-is, without re-calling the
    /// provider), advancing the position. Returns `Ok(None)` when there is
    /// nothing to replay, leaving the position untouched for a fresh execution.
    async fn decision_replay(
        &mut self,
        name: &str,
        _config: &DecisionConfig,
    ) -> Result<Option<DecisionOutput>, EngineError> {
        let position = self.position;

        let Some(existing) = self.replay_steps.get(&position).cloned() else {
            return Ok(None);
        };
        if existing.kind != StepKind::Decision {
            return Ok(None);
        }

        self.position += 1;

        let stored: DecisionOutput = existing
            .output
            .clone()
            .ok_or_else(|| {
                EngineError::StepConfig(format!(
                    "decision step '{name}' has no stored output to replay"
                ))
            })
            .and_then(|v| from_value(v).map_err(EngineError::from))?;

        // An escalated decision suspended in `AwaitingApproval`; the handler only
        // re-runs on an approved resume, so mark it completed and continue.
        if existing.status.state == StepStatus::AwaitingApproval {
            self.store
                .update_step(
                    existing.id,
                    StepUpdate {
                        status: Some(StepStatus::Completed),
                        completed_at: Some(Utc::now()),
                        ..StepUpdate::default()
                    },
                )
                .await?;
            info!(
                run_id = %self.run_id,
                step = %name,
                position,
                "decision step replayed (approved after escalation)"
            );
        } else {
            info!(
                run_id = %self.run_id,
                step = %name,
                position,
                "decision step replayed from previous execution"
            );
        }

        // Do not re-add cost/duration here. A replay only happens on a resume
        // within the same attempt, where `carry_over_run_totals` has already
        // seeded `total_cost_usd`/`total_duration_ms` from the run totals the
        // suspend snapshot persisted -- totals that already include this step.
        // Adding them again would double-count the escalated decision.
        self.last_step_ids = vec![existing.id];
        Ok(Some(stored))
    }

    /// Execute a fresh decision step: call the provider, persist the answers, and
    /// either complete or escalate to a human approval gate.
    async fn decision_execute(
        &mut self,
        name: &str,
        config: DecisionConfig,
    ) -> Result<DecisionOutput, EngineError> {
        self.check_guard_timeout()?;

        let position = self.position;
        self.position += 1;

        let provider =
            self.decision_provider
                .clone()
                .ok_or_else(|| EngineError::NoDecisionProvider {
                    step: name.to_string(),
                })?;

        let trace_id = step_trace_id(self.run_id, name, position);
        let step = self
            .store
            .create_step(NewStep {
                run_id: self.run_id,
                trace_id,
                name: name.to_string(),
                kind: StepKind::Decision,
                position,
                input: Some(to_value(&config)?),
                is_error_handler: false,
            })
            .await?;
        self.start_step(step.id, Utc::now()).await?;

        if let Some(ref bus) = self.event_bus {
            bus.publish(
                self.run_id,
                WorkflowEvent::StepStarted(WorkflowStepStartedEvent {
                    step_name: name.to_string(),
                    step_index: position,
                    timestamp: Utc::now(),
                }),
            );
        }

        let execution = match execute_decision(&provider, &config).await {
            Ok(execution) => execution,
            Err(err) => {
                self.fail_step(step.id, &err).await;
                return Err(err);
            }
        };

        // Impute cost and duration to the run, like an agent step.
        self.total_cost_usd += execution.cost_usd;
        self.total_duration_ms += execution.duration_ms;

        let output_value = to_value(&execution.output)?;

        let escalated = config
            .escalate_below
            .zip(execution.output.min_confidence())
            .map(|(threshold, min)| min < threshold)
            .unwrap_or(false);

        if escalated {
            return self
                .decision_escalate(name, position, step.id, &config, &execution, output_value)
                .await;
        }

        let step_output = StepOutput {
            output: output_value.clone(),
            duration_ms: execution.duration_ms,
            cost_usd: execution.cost_usd,
            input_tokens: Some(execution.input_tokens),
            cache_read_input_tokens: None,
            cache_creation_input_tokens: None,
            output_tokens: Some(execution.output_tokens),
            model: execution.output.model.as_ref().map(ToString::to_string),
            debug_messages: None,
        };

        let completed_at = Utc::now();
        self.store
            .update_step(
                step.id,
                StepUpdate {
                    status: Some(StepStatus::Completed),
                    output: Some(output_value),
                    duration_ms: Some(execution.duration_ms),
                    cost_usd: Some(execution.cost_usd),
                    input_tokens: Some(execution.input_tokens),
                    output_tokens: Some(execution.output_tokens),
                    completed_at: Some(completed_at),
                    ..StepUpdate::default()
                },
            )
            .await?;

        self.step_results
            .push(StepResult::from_success(trace_id, name, &step_output));
        self.persist_progress().await;
        self.last_step_ids = vec![step.id];

        info!(
            run_id = %self.run_id,
            step = %name,
            trace_id = %trace_id,
            cost_usd = %execution.cost_usd,
            "decision step completed"
        );

        if let Some(ref bus) = self.event_bus {
            bus.publish(
                self.run_id,
                WorkflowEvent::StepCompleted(WorkflowStepCompletedEvent {
                    step_name: name.to_string(),
                    step_index: position,
                    duration_ms: execution.duration_ms,
                    output_summary: None,
                }),
            );
        }

        Ok(execution.output)
    }

    /// Persist an escalated decision, suspend the run, and return
    /// [`EngineError::ApprovalRequired`].
    async fn decision_escalate(
        &mut self,
        name: &str,
        position: u32,
        step_id: Uuid,
        config: &DecisionConfig,
        execution: &DecisionExecution,
        output_value: Value,
    ) -> Result<DecisionOutput, EngineError> {
        let threshold = config.escalate_below.unwrap_or_default();
        let min = execution.output.min_confidence().unwrap_or_default();

        self.store
            .update_step(
                step_id,
                StepUpdate {
                    status: Some(StepStatus::AwaitingApproval),
                    output: Some(output_value),
                    duration_ms: Some(execution.duration_ms),
                    cost_usd: Some(execution.cost_usd),
                    input_tokens: Some(execution.input_tokens),
                    output_tokens: Some(execution.output_tokens),
                    ..StepUpdate::default()
                },
            )
            .await?;
        self.last_step_ids = vec![step_id];

        info!(
            run_id = %self.run_id,
            step = %name,
            position,
            confidence = min,
            threshold,
            "decision escalated to human approval"
        );

        if let Some(ref bus) = self.event_bus {
            bus.publish(
                self.run_id,
                WorkflowEvent::ApprovalRequired(WorkflowApprovalRequiredEvent {
                    step_name: name.to_string(),
                    step_index: position,
                    approval_id: step_id,
                }),
            );
        }

        Err(EngineError::ApprovalRequired {
            run_id: self.run_id,
            step_id,
            message: format!(
                "decision '{name}' escalated: confidence {min:.3} below threshold {threshold:.3}"
            ),
        })
    }
}
