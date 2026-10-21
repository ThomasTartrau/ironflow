//! Sub-workflow step for [`WorkflowContext`].
//!
//! A sub-workflow runs a registered [`WorkflowHandler`] in its own child run.
//! The child context is built here from the parent's private fields, which is
//! possible because this module is a descendant of `context`.
//!
//! A child that suspends (approval, human input, delay, signal) keeps its own
//! suspension status and the parent's `Workflow` step stays open with the
//! child run id in its output. The whole chain is suspended with it; when the
//! root run replays, the open step re-enters the same child run.
//!
//! With `allow_failure` (see [`WorkflowContext::workflow_with`]) a child whose
//! handler fails is still marked failed, but the parent's step completes with
//! the failure in its [`SubWorkflowOutput`].
//!
//! [`WorkflowContext::workflows_with`] records a wave of `Workflow` steps at
//! one position and runs their children concurrently (see [`wave`]).
//!
//! The child run itself is prepared in [`child`] and executed in
//! [`child_run`].

mod child;
mod child_run;
mod wave;

use chrono::Utc;
use rust_decimal::Decimal;
use serde_json::{Value, from_value, json, to_value};
use tracing::{error, info, warn};
use uuid::Uuid;

use ironflow_store::models::{
    NewStep, RunStatus, Step, StepKind, StepStatus, StepUpdate, normalize_worker_tags,
    step_trace_id,
};

use crate::config::{WorkflowOptions, WorkflowStepConfig};
use crate::context::WorkflowContext;
use crate::context::lifecycle::check_replay_identity;
use crate::error::EngineError;
use crate::executor::{
    ConcurrencyConflict, RecordedWorkflowStep, SubWorkflowOutcome, SubWorkflowOutput,
};
use crate::guard::WorkflowRejection;
use crate::handler::{TypedWorkflow, WorkflowHandler};
use crate::plan::{SharedPlanRecorder, lock_plan};

/// Key of the open `Workflow` step output that records the child run id.
const CHILD_RUN_ID_KEY: &str = "child_run_id";

/// The child run id recorded on an open or interrupted `Workflow` step, if any.
///
/// A missing or unparsable id means the parent stopped before the child run
/// was recorded: a new child run is started.
pub(crate) fn recorded_child_run_id(step: &Step) -> Option<Uuid> {
    let raw = step.output.as_ref()?.get(CHILD_RUN_ID_KEY)?.as_str()?;
    match Uuid::parse_str(raw) {
        Ok(id) => Some(id),
        Err(err) => {
            warn!(
                step_id = %step.id,
                value = %raw,
                error = %err,
                "open workflow step records an invalid child run id"
            );
            None
        }
    }
}

/// How a `Workflow` step reaches a child run it already started.
#[derive(Clone, Copy)]
enum ChildResume {
    /// The step was left open by a child that suspended.
    Suspended(Uuid),
    /// The step was interrupted by a lost worker lease: the child run may
    /// still hold the steps that were running in the dead worker.
    Interrupted(Uuid),
}

impl ChildResume {
    /// The child run to re-enter.
    fn run_id(self) -> Uuid {
        match self {
            Self::Suspended(id) | Self::Interrupted(id) => id,
        }
    }
}

/// How a child workflow execution ended, short of a suspension or an error.
enum ChildOutcome {
    /// The child run finished; the flag tells whether at least one
    /// `allow_failure` step failed.
    Finished(SubWorkflowOutput, bool),
    /// No child run was created: another active run holds the concurrency key.
    Conflict(ConcurrencyConflict),
}

/// Whether `err` stops the parent without closing the child's step: the next
/// replay re-enters the same child run.
fn leaves_step_open(err: &EngineError) -> bool {
    err.is_suspension() || matches!(err, EngineError::RunPaused { .. })
}

/// The output of a `workflow` or `workflow_dyn` step, which records no
/// concurrency key and so can only complete.
///
/// A conflict is only met on replay, when the step was recorded by
/// `workflow_with` before the handler code changed.
fn expect_completed(
    outcome: SubWorkflowOutcome,
    position: u32,
) -> Result<SubWorkflowOutput, EngineError> {
    match outcome {
        SubWorkflowOutcome::Completed(output) => Ok(output),
        SubWorkflowOutcome::Conflict(_) => Err(EngineError::StepConfig(format!(
            "workflow step at position {position} recorded a concurrency conflict; \
             call workflow_with to read it"
        ))),
    }
}

impl WorkflowContext {
    /// Execute a sub-workflow step.
    ///
    /// Creates a child run of `handler` whose payload is `input`, executes it
    /// with its own steps and lifecycle, and returns its run ID and aggregated
    /// metrics. The child declares its input type through [`TypedWorkflow`],
    /// so only a `W::Input` is accepted.
    ///
    /// When the child suspends (approval, human input, delay or signal), the
    /// parent is suspended with it and this step stays open. Resuming the
    /// child resumes the whole chain: the parent replays and re-enters the
    /// same child run, whose completed steps are replayed.
    ///
    /// To tolerate a failed child, see [`workflow_with`](Self::workflow_with).
    ///
    /// Requires the context to be created with
    /// `with_handler_resolver`.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::InvalidWorkflow`] if no handler is registered
    /// with the given name, or if no handler resolver is available, and
    /// [`EngineError::Serialization`] if `input` cannot be serialized. Returns
    /// [`EngineError::ReplayDivergence`] when the step recorded at this
    /// position has a different name or kind, and
    /// [`EngineError::ChildSuspended`] when the child run suspended.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::context::WorkflowContext;
    /// use ironflow_engine::error::EngineError;
    /// use ironflow_engine::handler::{HandlerFuture, TypedWorkflow, WorkflowHandler};
    /// use serde::{Deserialize, Serialize};
    ///
    /// #[derive(Serialize, Deserialize)]
    /// struct CollectInput {
    ///     scope: String,
    /// }
    ///
    /// struct Collect;
    ///
    /// impl WorkflowHandler for Collect {
    ///     fn name(&self) -> &str { "collect" }
    ///     fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
    ///         Box::pin(async move { Ok(()) })
    ///     }
    /// }
    ///
    /// impl TypedWorkflow for Collect {
    ///     type Input = CollectInput;
    /// }
    ///
    /// # async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    /// let child = ctx.workflow(&Collect, CollectInput { scope: "system".to_string() }).await?;
    /// let steps = ctx.store()?.list_steps(child.run_id()).await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// Any other input type is a compile error:
    ///
    /// ```compile_fail,E0308
    /// # use ironflow_engine::context::WorkflowContext;
    /// # use ironflow_engine::error::EngineError;
    /// # use ironflow_engine::handler::{HandlerFuture, TypedWorkflow, WorkflowHandler};
    /// # #[derive(serde::Serialize, serde::Deserialize)]
    /// # struct CollectInput { scope: String }
    /// # struct Collect;
    /// # impl WorkflowHandler for Collect {
    /// #     fn name(&self) -> &str { "collect" }
    /// #     fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
    /// #         Box::pin(async move { Ok(()) })
    /// #     }
    /// # }
    /// # impl TypedWorkflow for Collect { type Input = CollectInput; }
    /// # async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    /// ctx.workflow(&Collect, serde_json::json!({"scope": "system"})).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn workflow<W: TypedWorkflow>(
        &mut self,
        handler: &W,
        input: W::Input,
    ) -> Result<SubWorkflowOutput, EngineError> {
        let payload = to_value(&input)?;
        let position = self.position;
        let outcome = self
            .run_sub_workflow(handler, payload, WorkflowOptions::default())
            .await?;
        expect_completed(outcome, position)
    }

    /// Execute a sub-workflow step with [`WorkflowOptions`].
    ///
    /// Same as [`workflow`](Self::workflow), plus a concurrency key set with
    /// [`WorkflowOptions::concurrency_key`]: the key is held by the child run
    /// until it reaches a terminal state (Completed, Failed, Warning,
    /// Cancelled). While another non-terminal run holds it, no child run is
    /// created and the step completes at once with
    /// [`SubWorkflowOutcome::Conflict`], naming the run in place. A conflict
    /// never fails the parent, and it is replayed as-is on resume: the child
    /// is not attempted again.
    ///
    /// A parent that itself holds the key gets a conflict naming its own run.
    ///
    /// With [`allow_failure`](WorkflowOptions::allow_failure), a child whose
    /// handler fails does not fail the parent: the child run is still marked
    /// failed, but this step completes with a
    /// [`SubWorkflowOutcome::Completed`] whose [`SubWorkflowOutput`] has a
    /// [`status`](SubWorkflowOutput::status) of `Failed` (or `Cancelled` when a
    /// guardrail stopped it) and an [`error`](SubWorkflowOutput::error)
    /// carrying the child error. The parent run then ends as `Warning`. A
    /// resumed parent replays the completed step and creates no new child.
    ///
    /// A suspension is never tolerated: a child that suspends suspends the
    /// parent. Errors raised outside the child (no resolver, unknown handler,
    /// store errors, replay divergence, guard rejection of the invocation) are
    /// not tolerated either.
    ///
    /// # Errors
    ///
    /// Same as [`workflow`](Self::workflow), except that the failure of the
    /// child handler is returned in the output when `allow_failure` is set. A
    /// conflict raised while creating the child is data, not an error.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::config::WorkflowOptions;
    /// use ironflow_engine::context::WorkflowContext;
    /// use ironflow_engine::error::EngineError;
    /// use ironflow_engine::executor::SubWorkflowOutcome;
    /// use ironflow_engine::handler::{HandlerFuture, TypedWorkflow, WorkflowHandler};
    /// use serde::{Deserialize, Serialize};
    ///
    /// #[derive(Serialize, Deserialize)]
    /// struct FixInput {
    ///     issue: u64,
    /// }
    ///
    /// struct FixIssue;
    ///
    /// impl WorkflowHandler for FixIssue {
    ///     fn name(&self) -> &str { "fix-issue" }
    ///     fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
    ///         Box::pin(async move { Ok(()) })
    ///     }
    /// }
    ///
    /// impl TypedWorkflow for FixIssue {
    ///     type Input = FixInput;
    /// }
    ///
    /// # async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    /// let options = WorkflowOptions::new().concurrency_key("issue:12");
    /// match ctx.workflow_with(&FixIssue, FixInput { issue: 12 }, options).await? {
    ///     SubWorkflowOutcome::Completed(child) => println!("fixed in run {}", child.run_id()),
    ///     SubWorkflowOutcome::Conflict(c) => println!("already handled by run {}", c.run_id()),
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn workflow_with<W: TypedWorkflow>(
        &mut self,
        handler: &W,
        input: W::Input,
        options: WorkflowOptions,
    ) -> Result<SubWorkflowOutcome, EngineError> {
        let payload = to_value(&input)?;
        self.run_sub_workflow(handler, payload, options).await
    }

    /// Execute a sub-workflow step whose child is only known at run time.
    ///
    /// Same as [`workflow`](Self::workflow), without the compile-time check of
    /// the payload: the child must deserialize `payload` itself.
    ///
    /// # Errors
    ///
    /// Same as [`workflow`](Self::workflow).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::context::WorkflowContext;
    /// use ironflow_engine::error::EngineError;
    /// use ironflow_engine::handler::WorkflowHandler;
    /// use serde_json::json;
    ///
    /// # #[allow(deprecated)]
    /// # async fn example(ctx: &mut WorkflowContext, child: &dyn WorkflowHandler) -> Result<(), EngineError> {
    /// let result = ctx.workflow_dyn(child, json!({"scope": "system"})).await?;
    /// println!("child run {}", result.run_id());
    /// # Ok(())
    /// # }
    /// ```
    #[deprecated(
        note = "implement `TypedWorkflow` on the child and call `workflow`: its payload is then checked at compile time"
    )]
    pub async fn workflow_dyn(
        &mut self,
        handler: &dyn WorkflowHandler,
        payload: Value,
    ) -> Result<SubWorkflowOutput, EngineError> {
        let position = self.position;
        let outcome = self
            .run_sub_workflow(handler, payload, WorkflowOptions::default())
            .await?;
        expect_completed(outcome, position)
    }

    /// Record, then run or plan, a sub-workflow step.
    ///
    /// A `Workflow` step completed in a previous execution is replayed without
    /// running the child again. A step left open (`Running`) by a suspended
    /// child is reused and re-enters the child run it recorded.
    /// A step interrupted by a lost worker lease is recorded again at the same
    /// position and re-enters the child run the interrupted step recorded.
    async fn run_sub_workflow(
        &mut self,
        handler: &dyn WorkflowHandler,
        payload: Value,
        options: WorkflowOptions,
    ) -> Result<SubWorkflowOutcome, EngineError> {
        let (step_name, config) = options.into_step_config(handler.name(), payload);

        // Plan mode: record the invocation, expand the child handler in the
        // same recorder, and return a synthetic output. No child run is
        // created and no step of the child is executed.
        if let Some(plan) = self.plan().cloned() {
            let planned = self
                .plan_sub_workflow(&plan, handler, &step_name, config.payload)
                .await?;
            return Ok(SubWorkflowOutcome::Completed(planned));
        }

        let position = self.position;

        let existing = self.replay_steps.get(&position).cloned();
        if let Some(existing) = &existing {
            check_replay_identity(existing, position, &step_name, &StepKind::Workflow)?;
            if existing.status.state == StepStatus::Completed {
                return self.replay_sub_workflow(existing);
            }
        }

        // A step interrupted by a lost lease must be the same step the handler
        // calls now before its child run is re-entered.
        let interrupted = self
            .interrupted_children
            .get(&(position, step_name.clone()))
            .cloned();
        if let Some(interrupted) = &interrupted {
            check_replay_identity(interrupted, position, &step_name, &StepKind::Workflow)?;
        }

        // A child runs on the worker that runs its parent: refuse it here
        // rather than run it on a host missing what it requires.
        self.check_child_worker_tags(handler)?;

        // Guard: verify the limits and claim the invocation before creating the
        // step (fail-closed). The claim is atomic, so a sibling of a wave
        // running its own child at the same time cannot cross the fan-out too.
        if let (Some(guard_config), Some(guard_state)) = (&self.guard_config, &self.guard_state) {
            guard_state
                .lock()
                .map_err(|_| WorkflowRejection::GuardUnavailable)?
                .enter(guard_config, handler.name())?;
        }

        // Paused while it ran: no child is started, the run stops here.
        if let Err(err) = self.check_not_paused().await {
            self.guard_record_return();
            return Err(err);
        }

        self.position += 1;

        // An open step was left by a child that suspended: reuse it instead of
        // recording a second step at the same position.
        let (step_id, resume) = match existing.filter(|s| s.status.state == StepStatus::Running) {
            Some(step) => (
                step.id,
                recorded_child_run_id(&step).map(ChildResume::Suspended),
            ),
            None => {
                let recorded = self
                    .record_workflow_step(&step_name, &config, position)
                    .await;
                let step_id = match recorded {
                    Ok(step_id) => step_id,
                    Err(err) => {
                        self.guard_record_return();
                        return Err(err);
                    }
                };
                // A step interrupted by a lost lease re-enters the child run it
                // recorded instead of starting a new one.
                let resume = interrupted
                    .as_ref()
                    .and_then(recorded_child_run_id)
                    .map(ChildResume::Interrupted);
                (step_id, resume)
            }
        };

        let run = self.execute_child_workflow(&config, step_id, resume).await;
        if matches!(&run.result, Err(err) if err.is_suspension()) {
            self.sibling_suspensions.extend(run.sibling_suspensions);
        }
        let settled = self.settle_child_step(step_id, &config, run.result).await;
        self.guard_record_return();
        if settled.is_ok() {
            self.last_step_ids = vec![step_id];
        }
        settled
    }

    /// Record a `Workflow` step at `position` and start it.
    async fn record_workflow_step(
        &self,
        step_name: &str,
        config: &WorkflowStepConfig,
        position: u32,
    ) -> Result<Uuid, EngineError> {
        let step = self
            .store
            .create_step(NewStep {
                run_id: self.run_id,
                trace_id: step_trace_id(self.run_id, step_name, position),
                name: step_name.to_string(),
                kind: StepKind::Workflow,
                position,
                input: Some(to_value(config)?),
                is_error_handler: false,
            })
            .await?;
        self.start_step(step.id, Utc::now()).await?;
        Ok(step.id)
    }

    /// Close the `Workflow` step `step_id` from what its child produced, and
    /// account for the child on this run.
    ///
    /// A suspended or paused child leaves the step open, neither failed nor
    /// completed, so the next replay re-enters the same child run.
    async fn settle_child_step(
        &mut self,
        step_id: Uuid,
        config: &WorkflowStepConfig,
        result: Result<ChildOutcome, EngineError>,
    ) -> Result<SubWorkflowOutcome, EngineError> {
        match result {
            // No child run was created: the step completes with the conflict
            // as its output, so a replay serves the same outcome.
            Ok(ChildOutcome::Conflict(conflict)) => {
                self.store
                    .update_step(
                        step_id,
                        StepUpdate {
                            status: Some(StepStatus::Completed),
                            output: Some(json!({ "concurrency_conflict": conflict })),
                            duration_ms: Some(0),
                            cost_usd: Some(Decimal::ZERO),
                            completed_at: Some(Utc::now()),
                            ..StepUpdate::default()
                        },
                    )
                    .await?;

                info!(
                    run_id = %self.run_id,
                    child_workflow = %config.workflow_name,
                    key = %conflict.key(),
                    holder = %conflict.run_id(),
                    "workflow step skipped: concurrency conflict"
                );
                Ok(SubWorkflowOutcome::Conflict(conflict))
            }
            Ok(ChildOutcome::Finished(output, child_had_allowed_failure)) => {
                // Not `charge_cost`: a child sharing this run's live total
                // charged its cost there already, step by step.
                self.total_cost_usd += output.cost_usd();
                self.total_duration_ms += output.duration_ms();
                if child_had_allowed_failure {
                    self.has_allowed_failure = true;
                }

                self.store
                    .update_step(
                        step_id,
                        StepUpdate {
                            status: Some(StepStatus::Completed),
                            output: Some(to_value(&output)?),
                            duration_ms: Some(output.duration_ms()),
                            cost_usd: Some(output.cost_usd()),
                            completed_at: Some(Utc::now()),
                            ..StepUpdate::default()
                        },
                    )
                    .await?;

                info!(
                    run_id = %self.run_id,
                    child_workflow = %config.workflow_name,
                    duration_ms = output.duration_ms(),
                    "workflow step completed"
                );
                Ok(SubWorkflowOutcome::Completed(output))
            }
            Err(err) if leaves_step_open(&err) => Err(err),
            Err(err) => {
                if let Err(store_err) = self
                    .store
                    .update_step(
                        step_id,
                        StepUpdate {
                            status: Some(StepStatus::Failed),
                            error: Some(err.to_string()),
                            completed_at: Some(Utc::now()),
                            ..StepUpdate::default()
                        },
                    )
                    .await
                {
                    error!(step_id = %step_id, error = %store_err, "failed to persist step failure");
                }
                Err(err)
            }
        }
    }

    /// Replay a `Workflow` step completed in a previous execution: the child
    /// run is not executed again and nothing is re-counted by the guard.
    ///
    /// A step skipped on a concurrency conflict replays the same conflict: the
    /// child is not attempted again, even if the key has been released since.
    fn replay_sub_workflow(&mut self, step: &Step) -> Result<SubWorkflowOutcome, EngineError> {
        let outcome = self.replay_recorded_child(step)?;
        self.position += 1;
        self.last_step_ids = vec![step.id];
        Ok(outcome)
    }

    /// The outcome recorded on a completed `Workflow` step, accounted for on
    /// this run like the first time, without moving the position.
    fn replay_recorded_child(&mut self, step: &Step) -> Result<SubWorkflowOutcome, EngineError> {
        let recorded = step.output.clone().ok_or_else(|| {
            EngineError::StepConfig(format!(
                "completed workflow step {} has no recorded output",
                step.id
            ))
        })?;
        let recorded: RecordedWorkflowStep = from_value(recorded)?;

        let output = match SubWorkflowOutcome::from(recorded) {
            SubWorkflowOutcome::Completed(output) => output,
            SubWorkflowOutcome::Conflict(conflict) => {
                info!(
                    run_id = %self.run_id,
                    step = %step.name,
                    key = %conflict.key(),
                    holder = %conflict.run_id(),
                    "workflow step replayed: concurrency conflict"
                );
                return Ok(SubWorkflowOutcome::Conflict(conflict));
            }
        };

        // Cost is not added: `carry_over_run_totals` seeded `total_cost_usd`
        // from the run totals persisted before the suspension, which already
        // include this child.
        self.total_duration_ms += output.duration_ms();
        if matches!(
            output.status(),
            RunStatus::Warning | RunStatus::Failed | RunStatus::Cancelled
        ) {
            self.has_allowed_failure = true;
        }

        info!(
            run_id = %self.run_id,
            child_run_id = %output.run_id(),
            step = %step.name,
            "workflow step replayed from previous execution"
        );
        Ok(SubWorkflowOutcome::Completed(output))
    }

    /// Refuse a child whose required worker tags are not all carried by the
    /// worker running this context.
    ///
    /// No check is made outside a tagged worker (API or local mode).
    fn check_child_worker_tags(&self, handler: &dyn WorkflowHandler) -> Result<(), EngineError> {
        let Some(carried) = &self.worker_tags else {
            return Ok(());
        };
        let missing: Vec<String> = normalize_worker_tags(handler.required_worker_tags())
            .into_iter()
            .filter(|tag| !carried.contains(tag))
            .collect();
        if missing.is_empty() {
            return Ok(());
        }
        Err(EngineError::InvalidWorkflow(format!(
            "sub-workflow '{}' requires worker tags [{}] that this worker does not carry",
            handler.name(),
            missing.join(", ")
        )))
    }

    /// Record a sub-workflow invocation while planning, expanding the child
    /// handler into the same plan when the depth limit allows it.
    ///
    /// The child plans against its own payload and under its own workflow
    /// name; the parent's payload is restored on the way out.
    async fn plan_sub_workflow(
        &mut self,
        plan: &SharedPlanRecorder,
        handler: &dyn WorkflowHandler,
        step_name: &str,
        payload: Value,
    ) -> Result<SubWorkflowOutput, EngineError> {
        self.position += 1;
        {
            let mut recorder = lock_plan(plan);
            recorder.record_or_stop(step_name, StepKind::Workflow, &self.workflow_name, None)?;
            recorder.set_last(vec![step_name.to_string()]);
        }
        Ok(self.expand_planned_child(plan, handler, payload).await)
    }

    /// Expand `handler` into the plan, when the depth limit allows it, and
    /// return the synthetic output of its step.
    ///
    /// The child plans against its own payload and under its own workflow
    /// name; the parent's payload is restored on the way out.
    async fn expand_planned_child(
        &self,
        plan: &SharedPlanRecorder,
        handler: &dyn WorkflowHandler,
        payload: Value,
    ) -> SubWorkflowOutput {
        let sub_name = handler.name().to_string();
        // No child run exists while planning: a nil id and zero metrics.
        let planned = SubWorkflowOutput::new(
            Uuid::nil(),
            &sub_name,
            RunStatus::Completed,
            Decimal::ZERO,
            0,
        );

        let expand = lock_plan(plan).enter_workflow();
        if expand {
            let previous_payload = lock_plan(plan).swap_payload(payload.clone());

            let mut child = WorkflowContext::new(
                Uuid::now_v7(),
                sub_name.clone(),
                self.store.clone(),
                self.provider.clone(),
            );
            child.handler_resolver = self.handler_resolver.clone();
            child.set_plan(plan.clone());

            if let Err(err) = handler.execute(&mut child).await {
                lock_plan(plan).fail(format!(
                    "sub-workflow {sub_name} could not be planned: {err}"
                ));
            }

            let mut recorder = lock_plan(plan);
            recorder.swap_payload(previous_payload);
            recorder.leave_workflow();
        }

        planned
    }
}
