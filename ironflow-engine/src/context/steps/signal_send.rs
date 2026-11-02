//! Signal send step for [`WorkflowContext`].

use std::sync::Arc;
use std::time::Instant;

use chrono::Utc;
use rust_decimal::Decimal;
use serde_json::{from_value, json, to_value};
use tracing::info;
use uuid::Uuid;

use ironflow_store::models::{NewSignal, NewStep, StepKind, StepStatus, StepUpdate, step_trace_id};

use crate::context::WorkflowContext;
use crate::error::EngineError;
use crate::plan::lock_plan;
use crate::signal::{Signal, SignalDelivery, SignalOrigin, SignalSender};

impl WorkflowContext {
    /// Send a typed signal to the runs waiting for it, as a step of this run.
    ///
    /// The signal is named [`S::NAME`](Signal::NAME), carries `key` and has
    /// `signal` as payload: every run waiting on that `(name, key)` with
    /// [`wait_for_signal`](Self::wait_for_signal) and a matching schema
    /// resumes, exactly as with `POST /api/v1/signals`, without any API key.
    /// The step records the [`SignalDelivery`] and the audit log entry of the
    /// signal names this run and step as its sender.
    ///
    /// The handler must declare `S` in
    /// [`WorkflowHandler::sent_signals`](crate::handler::WorkflowHandler::sent_signals).
    /// The declaration guards against a workflow sending a signal that is not
    /// its own by mistake; it is not a security boundary. Anyone who can
    /// start this workflow chooses the key and payload it sends, so derive
    /// them from data you trust.
    ///
    /// A step that already completed is replayed without sending again. A
    /// step interrupted mid-delivery (worker lost) and executed again in the
    /// same attempt comes back with [`SignalDelivery::duplicate`] set instead
    /// of sending twice. A retry of the run is a new attempt: it sends again,
    /// like it runs a shell or HTTP step again.
    ///
    /// While planning, the step is recorded and nothing is sent.
    ///
    /// # Errors
    ///
    /// The step fails, and nothing is sent, with
    /// [`EngineError::SignalNotAllowed`] when the handler does not declare
    /// `S`, and [`EngineError::InvalidSignal`] when `key` is empty or longer
    /// than [`MAX_SIGNAL_FIELD_LEN`](crate::signal::MAX_SIGNAL_FIELD_LEN).
    /// Returns [`EngineError::ReplayDivergence`] when the step recorded at
    /// this position has a different name or kind, and other [`EngineError`]
    /// variants on store or delivery failures.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::context::WorkflowContext;
    /// use ironflow_engine::error::EngineError;
    /// use ironflow_engine::signal::Signal;
    /// use schemars::JsonSchema;
    /// use serde::{Deserialize, Serialize};
    ///
    /// #[derive(Serialize, Deserialize, JsonSchema)]
    /// struct HumanReplied {
    ///     body: String,
    /// }
    ///
    /// impl Signal for HumanReplied {
    ///     const NAME: &'static str = "gitlab.human_replied";
    /// }
    ///
    /// # async fn example(ctx: &mut WorkflowContext, discussion_id: &str) -> Result<(), EngineError> {
    /// let reply = HumanReplied { body: "ship it".to_string() };
    /// let delivery = ctx.send_signal("wake-waiting-run", &reply, discussion_id).await?;
    /// println!("{} runs resumed", delivery.resumed.len());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn send_signal<S: Signal>(
        &mut self,
        name: &str,
        signal: &S,
        key: &str,
    ) -> Result<SignalDelivery, EngineError> {
        let kind = StepKind::SignalSend;

        // Plan mode: record the step, send nothing.
        if let Some(plan) = self.plan().cloned() {
            self.position += 1;
            let mut recorder = lock_plan(&plan);
            recorder.record_or_stop(name, kind, &self.workflow_name, None)?;
            recorder.set_last(vec![name.to_string()]);
            return Ok(SignalDelivery {
                signal_id: Uuid::nil(),
                duplicate: false,
                resumed: Vec::new(),
                rejected: Vec::new(),
            });
        }

        let position = self.next_position();

        if let Some(replayed) = self.try_replay_step(position, name, &kind, false)? {
            return Ok(from_value(replayed.output)?);
        }

        let payload = to_value(signal)?;
        let step = self
            .store
            .create_step(NewStep {
                run_id: self.run_id,
                trace_id: step_trace_id(self.run_id, name, position),
                name: name.to_string(),
                kind,
                position,
                input: Some(json!({ "name": S::NAME, "key": key, "payload": payload })),
                is_error_handler: false,
            })
            .await?;
        self.start_step(step.id, Utc::now()).await?;

        let start = Instant::now();
        let new_signal = NewSignal {
            name: S::NAME.to_string(),
            key: key.to_string(),
            payload,
            // Positions repeat across attempts, never within one: a step
            // executed again after a lost worker reuses the key.
            idempotency_id: Some(format!(
                "run:{}:attempt:{}:step:{position}",
                self.run_id, self.attempt
            )),
        };
        let origin = SignalOrigin {
            run_id: self.run_id,
            step_id: step.id,
        };
        let delivered = match self.check_sent_signal(S::NAME) {
            Ok(sender) => sender.send(new_signal, origin).await,
            Err(err) => Err(err),
        };

        match delivered {
            Ok(delivery) => {
                let duration_ms = start.elapsed().as_millis() as u64;
                self.total_duration_ms += duration_ms;
                let step_id = self
                    .complete_step(
                        &step,
                        StepUpdate {
                            status: Some(StepStatus::Completed),
                            output: Some(to_value(&delivery)?),
                            duration_ms: Some(duration_ms),
                            cost_usd: Some(Decimal::ZERO),
                            completed_at: Some(Utc::now()),
                            ..StepUpdate::default()
                        },
                    )
                    .await?;
                self.last_step_ids = vec![step_id];
                info!(
                    run_id = %self.run_id,
                    step = %name,
                    signal = %S::NAME,
                    resumed = delivery.resumed.len(),
                    duplicate = delivery.duplicate,
                    "signal sent"
                );
                Ok(delivery)
            }
            Err(err) => {
                self.fail_step(step.id, &err).await;
                Err(err)
            }
        }
    }

    /// The sender to deliver `signal` with, once the handler declares it.
    fn check_sent_signal(&self, signal: &str) -> Result<Arc<dyn SignalSender>, EngineError> {
        let declared = self
            .handler_resolver
            .as_ref()
            .and_then(|resolve| resolve(&self.workflow_name))
            .is_some_and(|handler| handler.sent_signals().allows(signal));
        if !declared {
            return Err(EngineError::SignalNotAllowed {
                workflow: self.workflow_name.clone(),
                signal: signal.to_string(),
            });
        }
        self.signal_sender.clone().ok_or_else(|| {
            EngineError::StepConfig(
                "no signal sender: run this workflow through an Engine".to_string(),
            )
        })
    }
}
