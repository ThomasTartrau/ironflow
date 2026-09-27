//! Typed human input step for [`WorkflowContext`].

use chrono::{TimeDelta, Utc};
use schemars::{JsonSchema, schema_for};
use serde::de::DeserializeOwned;
use serde_json::{Value, from_value, json, to_value};
use tracing::info;

use ironflow_store::models::{NewStep, Step, StepKind, StepStatus, StepUpdate, step_trace_id};

use crate::config::{Approvers, HUMAN_INPUT_SCHEMA_KEY, HumanInputConfig};
use crate::context::WorkflowContext;
use crate::error::EngineError;
use crate::executor::HumanInputOutcome;
use crate::notify::{WorkflowEvent, WorkflowInputRequiredEvent};
use crate::plan::lock_plan;

impl WorkflowContext {
    /// Ask a human for a typed answer and suspend the run until it is given.
    ///
    /// On first execution, records a human input step carrying the JSON schema
    /// of `T` and returns [`EngineError::HumanInputRequired`] to suspend the
    /// run. The engine transitions the run to `AwaitingApproval`. The answer is
    /// posted to `POST /api/v1/runs/{id}/steps/{step_id}/input`, validated
    /// against the schema, and the run resumes.
    ///
    /// On resume, the step is replayed and the stored answer is deserialized
    /// into `T`. A rejected input (`POST .../steps/{step_id}/reject`) returns
    /// [`EngineError::HumanInputRejected`] so the handler decides what happens
    /// next. An answer given in an earlier attempt is carried over to a retry.
    ///
    /// The config reuses the approval gate machinery: deadline, escalation
    /// policy, assignee and the [`Approvers`] allowed to answer.
    ///
    /// While planning, the step is recorded and never suspends: `T` is
    /// deserialized from `{}` when it accepts that (for example with
    /// `#[serde(default)]`).
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::HumanInputRequired`] to pause the run until an
    /// answer is given. Returns [`EngineError::HumanInputRejected`] when the
    /// input was rejected. Returns [`EngineError::StepConfig`] when the stored
    /// answer does not match `T`, or while planning when `T` cannot be built
    /// from `{}`. Returns other [`EngineError`] variants on store failures.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::config::HumanInputConfig;
    /// use ironflow_engine::context::WorkflowContext;
    /// use ironflow_engine::error::EngineError;
    /// use schemars::JsonSchema;
    /// use serde::Deserialize;
    ///
    /// #[derive(Deserialize, JsonSchema)]
    /// struct Answers {
    ///     answers: Vec<String>,
    /// }
    ///
    /// # async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    /// let answers: Answers = ctx
    ///     .human_input("clarify", HumanInputConfig::new("Answer the clarification questions"))
    ///     .await?;
    /// assert!(answers.answers.len() < 100);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn human_input<T: DeserializeOwned + JsonSchema>(
        &mut self,
        name: &str,
        config: HumanInputConfig,
    ) -> Result<T, EngineError> {
        let schema = to_value(schema_for!(T))?;

        // Plan mode: record the step and continue. Planning must never suspend.
        if let Some(plan) = self.plan().cloned() {
            self.position += 1;
            {
                let mut recorder = lock_plan(&plan);
                if recorder.record(name, StepKind::HumanInput, &self.workflow_name, None) {
                    recorder.set_last(vec![name.to_string()]);
                }
            }
            return from_value(json!({})).map_err(|e| {
                EngineError::StepConfig(format!(
                    "human input '{name}' has no answer while planning: {e}"
                ))
            });
        }

        let value = self.human_input_value(name, &config, schema).await?;
        from_value::<T>(value).map_err(|e| {
            EngineError::StepConfig(format!(
                "human input '{name}' answer does not match the expected type: {e}"
            ))
        })
    }

    /// Replay, carry over, intercept or open a human input step and return the
    /// raw answer.
    async fn human_input_value(
        &mut self,
        name: &str,
        config: &HumanInputConfig,
        schema: Value,
    ) -> Result<Value, EngineError> {
        let position = self.position;
        self.position += 1;

        // Replay: the step exists from a prior execution of this attempt.
        if let Some(existing) = self
            .replay_steps
            .get(&position)
            .filter(|step| step.kind == StepKind::HumanInput)
            .cloned()
        {
            return self
                .human_input_replay(name, config, position, existing)
                .await;
        }

        // Carried over: a human already answered this input in an earlier
        // attempt. Record a fresh completed step so each attempt keeps a
        // complete DAG, and continue with the same answer.
        if let Some((answered_in, value)) = self.answered_inputs.get(&position).cloned() {
            let step = self
                .create_human_input_step(name, position, config, &schema)
                .await?;
            let now = Utc::now();
            self.start_step(step.id, now).await?;
            self.store
                .update_step(
                    step.id,
                    StepUpdate {
                        status: Some(StepStatus::Completed),
                        output: Some(value.clone()),
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
                answered_in_attempt = answered_in,
                attempt = self.attempt,
                "human input carried over from a previous attempt"
            );
            return Ok(value);
        }

        // Recorded only when the input opens: the stored requirement is the
        // source of truth from here.
        let requirement = config.approvers().map(Approvers::to_requirement);

        // An interceptor answers inline: the run neither suspends nor waits.
        if let Some(interceptor) = self.interceptor.clone()
            && let Some(outcome) = interceptor.intercept_human_input(name, config, &schema)
        {
            let step = self
                .create_human_input_step(name, position, config, &schema)
                .await?;
            let now = Utc::now();
            self.start_step(step.id, now).await?;
            self.last_step_ids = vec![step.id];

            return match outcome {
                HumanInputOutcome::Provided(value) => {
                    self.store
                        .update_step(
                            step.id,
                            StepUpdate {
                                status: Some(StepStatus::Completed),
                                output: Some(value.clone()),
                                approval_requirement: requirement,
                                completed_at: Some(now),
                                ..StepUpdate::default()
                            },
                        )
                        .await?;
                    info!(
                        run_id = %self.run_id,
                        step = %name,
                        position,
                        "human input provided by the step interceptor"
                    );
                    Ok(value)
                }
                HumanInputOutcome::Rejected { reason } => {
                    // The step FSM only reaches Rejected from AwaitingApproval.
                    self.store
                        .update_step(
                            step.id,
                            StepUpdate {
                                status: Some(StepStatus::AwaitingApproval),
                                approval_requirement: requirement,
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
                        "human input rejected by the step interceptor"
                    );
                    Err(EngineError::HumanInputRejected {
                        run_id: self.run_id,
                        step_id: step.id,
                        reason,
                    })
                }
            };
        }

        // First execution: create the step, arm the gate and suspend.
        let step = self
            .create_human_input_step(name, position, config, &schema)
            .await?;
        self.start_step(step.id, Utc::now()).await?;

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
                WorkflowEvent::InputRequired(WorkflowInputRequiredEvent {
                    run_id: self.run_id,
                    step_id: step.id,
                    step_name: name.to_string(),
                    step_index: position,
                    message: config.message().to_string(),
                    schema,
                }),
            );
        }

        info!(
            run_id = %self.run_id,
            step = %name,
            position,
            "human input requested"
        );
        Err(EngineError::HumanInputRequired {
            run_id: self.run_id,
            step_id: step.id,
            message: config.message().to_string(),
        })
    }

    /// Replay a human input step recorded in this attempt.
    async fn human_input_replay(
        &mut self,
        name: &str,
        config: &HumanInputConfig,
        position: u32,
        existing: Step,
    ) -> Result<Value, EngineError> {
        self.last_step_ids = vec![existing.id];

        match existing.status.state {
            StepStatus::Completed => {
                info!(
                    run_id = %self.run_id,
                    step = %name,
                    position,
                    "human input replayed (answered)"
                );
                existing.output.ok_or_else(|| {
                    EngineError::StepConfig(format!("human input '{name}' has no stored answer"))
                })
            }
            StepStatus::Rejected => {
                info!(
                    run_id = %self.run_id,
                    step = %name,
                    position,
                    "human input replayed (rejected)"
                );
                Err(EngineError::HumanInputRejected {
                    run_id: self.run_id,
                    step_id: existing.id,
                    reason: existing
                        .error
                        .unwrap_or_else(|| "input rejected".to_string()),
                })
            }
            state => {
                // Resumed without an answer (or after a crash mid-open): the
                // step keeps waiting, no new step is created.
                if state == StepStatus::Running {
                    self.store
                        .update_step(
                            existing.id,
                            StepUpdate {
                                status: Some(StepStatus::AwaitingApproval),
                                ..StepUpdate::default()
                            },
                        )
                        .await?;
                }
                info!(
                    run_id = %self.run_id,
                    step = %name,
                    position,
                    "human input still unanswered, suspending again"
                );
                Err(EngineError::HumanInputRequired {
                    run_id: self.run_id,
                    step_id: existing.id,
                    message: config.message().to_string(),
                })
            }
        }
    }

    /// Create the step record of a human input.
    async fn create_human_input_step(
        &self,
        name: &str,
        position: u32,
        config: &HumanInputConfig,
        schema: &Value,
    ) -> Result<Step, EngineError> {
        let trace_id = step_trace_id(self.run_id, name, position);
        Ok(self
            .store
            .create_step(NewStep {
                run_id: self.run_id,
                trace_id,
                name: name.to_string(),
                kind: StepKind::HumanInput,
                position,
                input: Some(stored_input(config, schema)?),
                is_error_handler: false,
            })
            .await?)
    }
}

/// The stored step input: the flattened config plus the answer schema under
/// [`HUMAN_INPUT_SCHEMA_KEY`].
fn stored_input(config: &HumanInputConfig, schema: &Value) -> Result<Value, EngineError> {
    let mut input = to_value(config)?;
    if let Some(object) = input.as_object_mut() {
        object.insert(HUMAN_INPUT_SCHEMA_KEY.to_string(), schema.clone());
    }
    Ok(input)
}
