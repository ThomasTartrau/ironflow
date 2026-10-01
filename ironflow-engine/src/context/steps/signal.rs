//! Signal wait step for [`WorkflowContext`].

use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use schemars::schema_for;
use serde_json::{Value, from_value, json, to_value};
use tracing::info;
use uuid::Uuid;

use ironflow_store::models::{
    NewStep, SignalStepResolution, Step, StepKind, StepStatus, StepUpdate, step_trace_id,
};

use crate::context::WorkflowContext;
use crate::context::lifecycle::check_replay_identity;
use crate::error::EngineError;
use crate::executor::SignalOutcome;
use crate::plan::lock_plan;
use crate::signal::{
    SIGNAL_SCHEMA_KEY, SIGNAL_TIMED_OUT_KEY, Signal, received_output, timed_out_output,
};

/// Key of the wait deadline in a signal step's input.
const DEADLINE_AT_KEY: &str = "deadline_at";

impl WorkflowContext {
    /// Wait for a typed external signal, suspending the run until it arrives
    /// or `timeout` elapses.
    ///
    /// The step waits for a signal named [`S::NAME`](Signal::NAME) carrying
    /// `key`. The key identifies one occurrence of the event, not the thing it
    /// happens to: wait on a commit SHA, not on a merge request, so a signal
    /// for an older push never resumes a run waiting for the newest one.
    ///
    /// A signal received since the run was created resolves the step at once,
    /// without suspending. Otherwise the step stores the JSON schema of `S`,
    /// the run goes `Sleeping` until the deadline, and a delivery
    /// (`POST /api/v1/signals`,
    /// [`Engine::send_signal`](crate::engine::Engine::send_signal)) whose
    /// payload matches the schema wakes it. On resume the step is replayed:
    /// the payload is deserialized into `S` and returned as `Some`. When the
    /// deadline passes first, the step completes as timed out and `None` is
    /// returned.
    ///
    /// While planning, the step is recorded and never suspends: `S` is
    /// deserialized from `{}` when it accepts that, `None` otherwise.
    ///
    /// # Panics
    ///
    /// Panics when `key` is empty or whitespace, when `timeout` is zero, or
    /// when `S::NAME` is empty.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::SignalWaiting`] to suspend the run until a signal
    /// or the deadline. Returns [`EngineError::StepConfig`] when the stored
    /// payload does not match `S` or the timeout is out of range. Returns
    /// [`EngineError::ReplayDivergence`] when the step recorded at this
    /// position has a different name or kind. Returns other [`EngineError`]
    /// variants on store failures.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::time::Duration;
    ///
    /// use ironflow_engine::context::WorkflowContext;
    /// use ironflow_engine::error::EngineError;
    /// use ironflow_engine::signal::Signal;
    /// use schemars::JsonSchema;
    /// use serde::{Deserialize, Serialize};
    ///
    /// #[derive(Serialize, Deserialize, JsonSchema)]
    /// struct PipelineFinished {
    ///     status: String,
    /// }
    ///
    /// impl Signal for PipelineFinished {
    ///     const NAME: &'static str = "ci.pipeline_finished";
    /// }
    ///
    /// # async fn example(ctx: &mut WorkflowContext, sha: &str) -> Result<(), EngineError> {
    /// let finished = ctx
    ///     .wait_for_signal::<PipelineFinished>("wait-ci", sha, Duration::from_secs(3600))
    ///     .await?;
    /// match finished {
    ///     Some(pipeline) if pipeline.status == "success" => { /* deploy */ }
    ///     Some(_) => return Err(EngineError::StepConfig("CI failed".to_string())),
    ///     None => return Err(EngineError::StepConfig("CI timed out".to_string())),
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn wait_for_signal<S: Signal>(
        &mut self,
        name: &str,
        key: &str,
        timeout: Duration,
    ) -> Result<Option<S>, EngineError> {
        assert!(
            !S::NAME.is_empty(),
            "wait_for_signal: Signal::NAME must not be empty"
        );
        assert!(
            !key.trim().is_empty(),
            "wait_for_signal: key must not be empty"
        );
        assert!(
            !timeout.is_zero(),
            "wait_for_signal: timeout must be greater than zero"
        );

        // Plan mode: record the step and continue. Planning must never suspend.
        if let Some(plan) = self.plan().cloned() {
            self.position += 1;
            {
                let mut recorder = lock_plan(&plan);
                if recorder.record(name, StepKind::Signal, &self.workflow_name, None) {
                    recorder.set_last(vec![name.to_string()]);
                }
            }
            return Ok(from_value::<S>(json!({})).ok());
        }

        let position = self.next_position();

        // Replay: the step exists from a prior execution of this attempt.
        if let Some(existing) = self.replay_steps.get(&position).cloned() {
            check_replay_identity(&existing, position, name, &StepKind::Signal)?;
            self.last_step_ids = vec![existing.id];
            return self.signal_replay::<S>(name, key, existing).await;
        }

        let schema = to_value(schema_for!(S))?;
        let now = Utc::now();
        let deadline_at = deadline(name, now, timeout)?;
        let input = json!({
            "name": S::NAME,
            "key": key,
            SIGNAL_SCHEMA_KEY: schema,
            "waiting_since": now,
            DEADLINE_AT_KEY: deadline_at,
        });

        // An interceptor resolves the step inline: the run never suspends.
        if let Some(interceptor) = self.interceptor.clone()
            && let Some(outcome) = interceptor.intercept_signal(name, S::NAME, key, &schema)
        {
            let step = self.create_signal_step(name, position, input).await?;
            self.start_step(step.id, now).await?;
            self.last_step_ids = vec![step.id];

            let output = match outcome {
                SignalOutcome::Received(payload) => json!({
                    SIGNAL_TIMED_OUT_KEY: false,
                    "signal_id": null,
                    "payload": payload,
                }),
                SignalOutcome::TimedOut => timed_out_output(),
            };
            self.store
                .update_step(
                    step.id,
                    StepUpdate {
                        status: Some(StepStatus::Completed),
                        output: Some(output.clone()),
                        completed_at: Some(Utc::now()),
                        ..StepUpdate::default()
                    },
                )
                .await?;
            info!(
                run_id = %self.run_id,
                step = %name,
                position,
                "signal step resolved by the step interceptor"
            );
            return decode_signal_output(name, &output);
        }

        // First execution: open the step. Once `Running` it is visible to
        // deliveries, so a signal sent from here on resolves it.
        let step = self.create_signal_step(name, position, input).await?;
        self.start_step(step.id, now).await?;
        self.last_step_ids = vec![step.id];

        if let Some(output) = self.received_signal::<S>(step.id, key).await? {
            info!(
                run_id = %self.run_id,
                step = %name,
                position,
                "signal already received, not suspending"
            );
            return decode_signal_output(name, &output);
        }

        info!(
            run_id = %self.run_id,
            step = %name,
            signal = %S::NAME,
            key = %key,
            deadline_at = %deadline_at,
            "waiting for signal"
        );
        Err(EngineError::SignalWaiting {
            run_id: self.run_id,
            step_id: step.id,
            step_name: name.to_string(),
            name: S::NAME.to_string(),
            key: key.to_string(),
            deadline_at,
        })
    }

    /// Replay a signal step recorded in this attempt.
    async fn signal_replay<S: Signal>(
        &mut self,
        name: &str,
        key: &str,
        existing: Step,
    ) -> Result<Option<S>, EngineError> {
        match existing.status.state {
            StepStatus::Completed => {
                let output = require_output(name, existing.output)?;
                info!(
                    run_id = %self.run_id,
                    step = %name,
                    "signal step replayed (resolved)"
                );
                decode_signal_output(name, &output)
            }
            StepStatus::Running => {
                let deadline_at = stored_deadline(name, existing.input.as_ref())?;

                if Utc::now() >= deadline_at {
                    let resolution = self
                        .store
                        .resolve_signal_step(existing.id, timed_out_output())
                        .await?;
                    let output = match resolution {
                        SignalStepResolution::Resolved { .. } => timed_out_output(),
                        // A delivery won the race against the timeout.
                        SignalStepResolution::NotWaiting { output } => {
                            require_output(name, output)?
                        }
                    };
                    info!(
                        run_id = %self.run_id,
                        step = %name,
                        "signal step deadline reached"
                    );
                    return decode_signal_output(name, &output);
                }

                // Woken before the deadline without a delivery on the step:
                // look once more, then keep waiting until the same deadline.
                if let Some(output) = self.received_signal::<S>(existing.id, key).await? {
                    return decode_signal_output(name, &output);
                }
                info!(
                    run_id = %self.run_id,
                    step = %name,
                    deadline_at = %deadline_at,
                    "signal still not received, suspending again"
                );
                Err(EngineError::SignalWaiting {
                    run_id: self.run_id,
                    step_id: existing.id,
                    step_name: name.to_string(),
                    name: S::NAME.to_string(),
                    key: key.to_string(),
                    deadline_at,
                })
            }
            state => Err(EngineError::StepConfig(format!(
                "signal step '{name}' is in state {state:?}"
            ))),
        }
    }

    /// Resolve `step_id` with the oldest signal received for `key` since the
    /// run was created whose payload matches `S`.
    ///
    /// Returns the output recorded on the step, or `None` when no such signal
    /// exists.
    async fn received_signal<S: Signal>(
        &self,
        step_id: Uuid,
        key: &str,
    ) -> Result<Option<Value>, EngineError> {
        let since = self.run_created_at().await?;
        let signals = self.store.list_signals_for_key(S::NAME, key, since).await?;
        let Some(signal) = signals
            .iter()
            .find(|s| from_value::<S>(s.payload.clone()).is_ok())
        else {
            return Ok(None);
        };

        let output = received_output(signal);
        let resolution = self
            .store
            .resolve_signal_step(step_id, output.clone())
            .await?;
        match resolution {
            SignalStepResolution::Resolved { .. } => Ok(Some(output)),
            // A concurrent delivery resolved the step first: its output wins.
            SignalStepResolution::NotWaiting { output } => Ok(output),
        }
    }

    /// Create the step record of a signal wait.
    async fn create_signal_step(
        &self,
        name: &str,
        position: u32,
        input: Value,
    ) -> Result<Step, EngineError> {
        let trace_id = step_trace_id(self.run_id, name, position);
        Ok(self
            .store
            .create_step(NewStep {
                run_id: self.run_id,
                trace_id,
                name: name.to_string(),
                kind: StepKind::Signal,
                position,
                input: Some(input),
                is_error_handler: false,
            })
            .await?)
    }
}

/// The wait deadline: `now + timeout`.
fn deadline(
    name: &str,
    now: DateTime<Utc>,
    timeout: Duration,
) -> Result<DateTime<Utc>, EngineError> {
    let delta = TimeDelta::from_std(timeout).ok();
    match delta.and_then(|delta| now.checked_add_signed(delta)) {
        Some(deadline_at) => Ok(deadline_at),
        None => Err(EngineError::StepConfig(format!(
            "signal step '{name}' timeout {timeout:?} is out of range"
        ))),
    }
}

/// The deadline stored in a signal step's input when it opened.
fn stored_deadline(name: &str, input: Option<&Value>) -> Result<DateTime<Utc>, EngineError> {
    let Some(value) = input.and_then(|i| i.get(DEADLINE_AT_KEY)) else {
        let message = format!("signal step '{name}' has no stored deadline");
        return Err(EngineError::StepConfig(message));
    };
    match from_value(value.clone()) {
        Ok(deadline_at) => Ok(deadline_at),
        Err(e) => Err(EngineError::StepConfig(format!(
            "signal step '{name}' has an invalid deadline: {e}"
        ))),
    }
}

/// The output recorded on a resolved signal step.
fn require_output(name: &str, output: Option<Value>) -> Result<Value, EngineError> {
    let Some(output) = output else {
        let message = format!("signal step '{name}' has no stored output");
        return Err(EngineError::StepConfig(message));
    };
    Ok(output)
}

/// Turn a signal step output into the handler's result: `None` on timeout,
/// the payload deserialized into `S` otherwise.
fn decode_signal_output<S: Signal>(name: &str, output: &Value) -> Result<Option<S>, EngineError> {
    if output.get(SIGNAL_TIMED_OUT_KEY).and_then(Value::as_bool) == Some(true) {
        return Ok(None);
    }
    let Some(payload) = output.get("payload") else {
        let message = format!("signal step '{name}' output has no payload");
        return Err(EngineError::StepConfig(message));
    };
    match from_value::<S>(payload.clone()) {
        Ok(signal) => Ok(Some(signal)),
        Err(e) => Err(EngineError::StepConfig(format!(
            "signal step '{name}' payload does not match the expected type: {e}"
        ))),
    }
}
