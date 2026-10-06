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

use std::collections::HashMap;
use std::time::Instant;

use chrono::Utc;
use rust_decimal::Decimal;
use serde_json::{Value, from_value, json, to_value};
use tracing::{error, info, warn};
use uuid::Uuid;

use ironflow_core::provider::LABEL_ROOT_RUN_ID;
use ironflow_store::error::StoreError;
use ironflow_store::models::{
    NewRun, NewStep, Run, RunStatus, RunUpdate, Step, StepKind, StepStatus, StepUpdate,
    TriggerKind, step_trace_id,
};

use crate::config::{WorkflowOptions, WorkflowStepConfig};
use crate::context::lifecycle::check_replay_identity;
use crate::context::{PARENT_RUN_ID_LABEL, WorkflowContext, interrupt_running_steps};
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
pub(in crate::context) fn recorded_child_run_id(step: &Step) -> Option<Uuid> {
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

/// The outcome of a child run found `Cancelled`: with `allow_failure`, a
/// `Cancelled` output carrying the child's error; otherwise
/// [`EngineError::ChildRunCancelled`], which fails the step and the parent.
fn cancelled_child_outcome(
    config: &WorkflowStepConfig,
    child: &Run,
    cost_usd: Decimal,
    duration_ms: u64,
    output: Option<Value>,
) -> Result<ChildOutcome, EngineError> {
    let cancelled = EngineError::ChildRunCancelled { run_id: child.id };
    if !config.allow_failure {
        return Err(cancelled);
    }
    let error = child.error.clone().unwrap_or_else(|| cancelled.to_string());
    let status = RunStatus::Cancelled;
    Ok(ChildOutcome::Finished(
        SubWorkflowOutput::new(
            child.id,
            &config.workflow_name,
            status,
            cost_usd,
            duration_ms,
        )
        .with_output(output)
        .with_error(error),
        true,
    ))
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
    /// let steps = ctx.store().list_steps(child.run_id()).await?;
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
        // Plan mode: record the invocation, expand the child handler in the
        // same recorder, and return a synthetic output. No child run is
        // created and no step of the child is executed.
        if let Some(plan) = self.plan().cloned() {
            let planned = self.plan_sub_workflow(&plan, handler, payload).await?;
            return Ok(SubWorkflowOutcome::Completed(planned));
        }

        let mut config = WorkflowStepConfig::new(handler.name(), payload);
        config.allow_failure = options.allow_failure;
        config.concurrency_key = options.into_concurrency_key();
        let position = self.position;

        let existing = self.replay_steps.get(&position).cloned();
        if let Some(existing) = &existing {
            check_replay_identity(
                existing,
                position,
                &config.workflow_name,
                &StepKind::Workflow,
            )?;
            if existing.status.state == StepStatus::Completed {
                return self.replay_sub_workflow(existing);
            }
        }

        // A step interrupted by a lost lease must be the same step the handler
        // calls now before its child run is re-entered.
        let interrupted = self.interrupted_children.get(&position).cloned();
        if let Some(interrupted) = &interrupted {
            check_replay_identity(
                interrupted,
                position,
                &config.workflow_name,
                &StepKind::Workflow,
            )?;
        }

        // Guard check: verify limits before creating the step.
        if let (Some(guard_config), Some(guard_state)) = (&self.guard_config, &self.guard_state) {
            let state = guard_state
                .lock()
                .map_err(|_| WorkflowRejection::GuardUnavailable)?;
            state.check(guard_config, handler.name())?;
        }

        self.position += 1;

        // An open step was left by a child that suspended: reuse it instead of
        // recording a second step at the same position.
        let (step, resume) = match existing.filter(|s| s.status.state == StepStatus::Running) {
            Some(step) => {
                let resume = recorded_child_run_id(&step).map(ChildResume::Suspended);
                (step, resume)
            }
            None => {
                let trace_id = step_trace_id(self.run_id, &config.workflow_name, position);
                let step = self
                    .store
                    .create_step(NewStep {
                        run_id: self.run_id,
                        trace_id,
                        name: config.workflow_name.clone(),
                        kind: StepKind::Workflow,
                        position,
                        input: Some(to_value(&config)?),
                        is_error_handler: false,
                    })
                    .await?;

                self.start_step(step.id, Utc::now()).await?;
                // A step interrupted by a lost lease re-enters the child run it
                // recorded instead of starting a new one.
                let resume = interrupted
                    .as_ref()
                    .and_then(recorded_child_run_id)
                    .map(ChildResume::Interrupted);
                (step, resume)
            }
        };

        // Record invocation in guard state (fail-closed).
        if let Some(guard_state) = &self.guard_state {
            let mut state = guard_state
                .lock()
                .map_err(|_| WorkflowRejection::GuardUnavailable)?;
            state.record_invocation(handler.name());
        }

        match self.execute_child_workflow(&config, step.id, resume).await {
            // No child run was created: the step completes with the conflict
            // as its output, so a replay serves the same outcome.
            Ok(ChildOutcome::Conflict(conflict)) => {
                self.store
                    .update_step(
                        step.id,
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

                self.last_step_ids = vec![step.id];

                self.guard_record_return();
                Ok(SubWorkflowOutcome::Conflict(conflict))
            }
            Ok(ChildOutcome::Finished(output, child_had_allowed_failure)) => {
                self.total_cost_usd += output.cost_usd();
                self.total_duration_ms += output.duration_ms();
                if child_had_allowed_failure {
                    self.has_allowed_failure = true;
                }

                let completed_at = Utc::now();
                self.store
                    .update_step(
                        step.id,
                        StepUpdate {
                            status: Some(StepStatus::Completed),
                            output: Some(to_value(&output)?),
                            duration_ms: Some(output.duration_ms()),
                            cost_usd: Some(output.cost_usd()),
                            completed_at: Some(completed_at),
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

                self.last_step_ids = vec![step.id];

                self.guard_record_return();
                Ok(SubWorkflowOutcome::Completed(output))
            }
            // The child suspended: the step stays open, neither failed nor
            // completed, so the next replay re-enters the same child run.
            Err(err) if err.is_suspension() => {
                self.guard_record_return();
                Err(err)
            }
            Err(err) => {
                let completed_at = Utc::now();
                if let Err(store_err) = self
                    .store
                    .update_step(
                        step.id,
                        StepUpdate {
                            status: Some(StepStatus::Failed),
                            error: Some(err.to_string()),
                            completed_at: Some(completed_at),
                            ..StepUpdate::default()
                        },
                    )
                    .await
                {
                    error!(step_id = %step.id, error = %store_err, "failed to persist step failure");
                }

                self.guard_record_return();
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
        let recorded = step.output.clone().ok_or_else(|| {
            EngineError::StepConfig(format!(
                "completed workflow step {} has no recorded output",
                step.id
            ))
        })?;
        let recorded: RecordedWorkflowStep = from_value(recorded)?;

        self.position += 1;
        self.last_step_ids = vec![step.id];

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

    /// Record a sub-workflow invocation while planning, expanding the child
    /// handler into the same plan when the depth limit allows it.
    ///
    /// The child plans against its own payload and under its own workflow
    /// name; the parent's payload is restored on the way out.
    async fn plan_sub_workflow(
        &mut self,
        plan: &SharedPlanRecorder,
        handler: &dyn WorkflowHandler,
        payload: Value,
    ) -> Result<SubWorkflowOutput, EngineError> {
        self.position += 1;
        let sub_name = handler.name().to_string();
        // No child run exists while planning: a nil id and zero metrics.
        let planned = SubWorkflowOutput::new(
            Uuid::nil(),
            &sub_name,
            RunStatus::Completed,
            Decimal::ZERO,
            0,
        );

        {
            let mut recorder = lock_plan(plan);
            if !recorder.record(&sub_name, StepKind::Workflow, &self.workflow_name, None) {
                return Ok(planned);
            }
            recorder.set_last(vec![sub_name.clone()]);
        }

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

        Ok(planned)
    }

    /// Execute a child workflow and return aggregated output plus whether
    /// at least one `allow_failure` step failed.
    ///
    /// When another active run holds the step's concurrency key, no child run
    /// is created and [`ChildOutcome::Conflict`] is returned.
    ///
    /// `resume` is the child run recorded on an open or interrupted step: that
    /// run is re-entered, with its completed steps replayed, instead of
    /// creating a new one. A child re-entered after a lost lease has its
    /// `Running` steps marked interrupted first, like a requeued run, and the
    /// new step records the same child run so a second interruption re-enters
    /// it again. A child that suspends is left in its suspension status and
    /// [`EngineError::ChildSuspended`] is returned.
    async fn execute_child_workflow(
        &self,
        config: &WorkflowStepConfig,
        step_id: Uuid,
        resume: Option<ChildResume>,
    ) -> Result<ChildOutcome, EngineError> {
        let resolver = self.handler_resolver.as_ref().ok_or_else(|| {
            EngineError::InvalidWorkflow(
                "sub-workflow requires a handler resolver (use Engine to execute)".to_string(),
            )
        })?;

        let handler = resolver(&config.workflow_name).ok_or_else(|| {
            EngineError::InvalidWorkflow(format!("no handler registered: {}", config.workflow_name))
        })?;

        let (child_run_id, carried_cost_usd, carried_duration_ms) = match resume {
            Some(resume) => {
                let child_run_id = resume.run_id();
                if let ChildResume::Interrupted(_) = resume {
                    self.store
                        .update_step(
                            step_id,
                            StepUpdate {
                                output: Some(json!({ CHILD_RUN_ID_KEY: child_run_id })),
                                ..StepUpdate::default()
                            },
                        )
                        .await?;
                }

                let child_run = self
                    .store
                    .get_run(child_run_id)
                    .await?
                    .ok_or(EngineError::Store(StoreError::RunNotFound(child_run_id)))?;

                match child_run.status.state {
                    // Left running by the worker that lost the lease: its open
                    // steps are executed again, like those of a requeued run.
                    RunStatus::Running if matches!(resume, ChildResume::Interrupted(_)) => {
                        interrupt_running_steps(self.store.as_ref(), child_run_id).await?;
                    }
                    // Already moved to Running by the path that resumed it.
                    RunStatus::Running => {}
                    RunStatus::AwaitingApproval | RunStatus::Pending => {
                        self.store
                            .update_run_status(child_run_id, RunStatus::Running)
                            .await?;
                    }
                    RunStatus::Sleeping => {
                        self.store
                            .update_run_status(child_run_id, RunStatus::Pending)
                            .await?;
                        self.store
                            .update_run_status(child_run_id, RunStatus::Running)
                            .await?;
                    }
                    // Cancelled while the chain waited, or before the parent
                    // closed its step: the cancellation is the outcome.
                    RunStatus::Cancelled => {
                        return cancelled_child_outcome(
                            config,
                            &child_run,
                            child_run.cost_usd,
                            child_run.duration_ms,
                            child_run.output.clone(),
                        );
                    }
                    // The child finished but the parent stopped before closing
                    // its step: report the recorded outcome, run nothing.
                    status @ RunStatus::Failed if config.allow_failure => {
                        let error = child_run
                            .error
                            .clone()
                            .unwrap_or_else(|| "child run failed".to_string());
                        return Ok(ChildOutcome::Finished(
                            SubWorkflowOutput::new(
                                child_run_id,
                                &config.workflow_name,
                                status,
                                child_run.cost_usd,
                                child_run.duration_ms,
                            )
                            .with_output(child_run.output.clone())
                            .with_error(error),
                            true,
                        ));
                    }
                    status @ (RunStatus::Completed | RunStatus::Warning) => {
                        return Ok(ChildOutcome::Finished(
                            SubWorkflowOutput::new(
                                child_run_id,
                                &config.workflow_name,
                                status,
                                child_run.cost_usd,
                                child_run.duration_ms,
                            )
                            .with_output(child_run.output.clone()),
                            status == RunStatus::Warning,
                        ));
                    }
                    other => {
                        return Err(EngineError::InvalidWorkflow(format!(
                            "child run {child_run_id} is {other}"
                        )));
                    }
                }

                info!(
                    parent_run_id = %self.run_id,
                    child_run_id = %child_run_id,
                    workflow = %config.workflow_name,
                    "child run re-entered"
                );
                (child_run_id, child_run.cost_usd, child_run.duration_ms)
            }
            None => {
                let child_run_id = match self.create_child_run(config).await {
                    Ok(id) => id,
                    Err(EngineError::ConcurrencyConflict { key, run_id }) => {
                        return Ok(ChildOutcome::Conflict(ConcurrencyConflict::new(
                            key, run_id,
                        )));
                    }
                    Err(err) => return Err(err),
                };

                // Recorded before the child runs, so a suspension of the child
                // can be resumed into this same run.
                self.store
                    .update_step(
                        step_id,
                        StepUpdate {
                            output: Some(json!({ CHILD_RUN_ID_KEY: child_run_id })),
                            ..StepUpdate::default()
                        },
                    )
                    .await?;

                self.store
                    .update_run_status(child_run_id, RunStatus::Running)
                    .await?;
                (child_run_id, Decimal::ZERO, 0)
            }
        };

        let run_start = Instant::now();
        let mut child_ctx = WorkflowContext {
            run_id: child_run_id,
            root_run_id: self.root_run_id,
            workflow_name: config.workflow_name.clone(),
            store: self.store.clone(),
            provider: self.provider.clone(),
            decision_provider: self.decision_provider.clone(),
            handler_resolver: self.handler_resolver.clone(),
            position: 0,
            last_step_ids: Vec::new(),
            // A re-entered child starts from what it already spent, like a
            // resumed top-level run.
            total_cost_usd: carried_cost_usd,
            total_duration_ms: 0,
            max_cost_usd: self.max_cost_usd,
            // Everything the parent chain already spent counts against the
            // shared cap, so the child cannot restart the budget from zero.
            inherited_cost_usd: self.charged_cost_usd(),
            replay_steps: HashMap::new(),
            replay_wave_steps: HashMap::new(),
            granted_approvals: HashMap::new(),
            answered_inputs: HashMap::new(),
            interrupted_children: HashMap::new(),
            // A child run is never itself retried.
            attempt: 1,
            carried_duration_ms,
            log_sender: self.log_sender.clone(),
            // A child shares the storage backend but not the parent's artifacts:
            // input lookups are scoped to the child's own run.
            artifact_sink: self.artifact_sink.clone(),
            has_allowed_failure: false,
            error_handlers: Vec::new(),
            guard_state: self.guard_state.clone(),
            guard_config: self.guard_config.clone(),
            step_results: Vec::new(),
            event_bus: self.event_bus.clone(),
            // A child run is mocked exactly like its parent.
            interceptor: self.interceptor.clone(),
            trace_context: self.trace_context.child(),
            operation_ctx: None,
            run_created_at: None,
            plan: None,
            output: None,
        };

        // A re-entered child replays its completed steps and is served the
        // answer, signal or elapsed delay it was suspended on.
        let loaded = if resume.is_some() {
            child_ctx.load_replay_steps().await
        } else {
            Ok(())
        };
        let result = match loaded {
            Ok(()) => handler.execute(&mut child_ctx).await,
            Err(err) => Err(err),
        };
        let total_duration = child_ctx.carried_duration_ms + run_start.elapsed().as_millis() as u64;
        let completed_at = Utc::now();

        // Cancelled while it ran: whatever the handler returned, the child
        // stays cancelled and the cancellation is its outcome.
        if let Some(child_run) = self.store.get_run(child_run_id).await?
            && child_run.status.state == RunStatus::Cancelled
        {
            return cancelled_child_outcome(
                config,
                &child_run,
                child_ctx.total_cost_usd,
                total_duration,
                child_ctx.output().cloned(),
            );
        }

        match result {
            Ok(()) => {
                let child_status = if child_ctx.has_allowed_failure {
                    RunStatus::Warning
                } else {
                    RunStatus::Completed
                };
                self.store
                    .update_run(
                        child_run_id,
                        RunUpdate {
                            status: Some(child_status),
                            cost_usd: Some(child_ctx.total_cost_usd),
                            duration_ms: Some(total_duration),
                            completed_at: Some(completed_at),
                            output: child_ctx.output().cloned(),
                            ..RunUpdate::default()
                        },
                    )
                    .await?;

                let child_had_allowed_failure = child_ctx.has_allowed_failure;
                Ok(ChildOutcome::Finished(
                    SubWorkflowOutput::new(
                        child_run_id,
                        &config.workflow_name,
                        child_status,
                        child_ctx.total_cost_usd,
                        total_duration,
                    )
                    .with_output(child_ctx.output().cloned()),
                    child_had_allowed_failure,
                ))
            }
            Err(err) if err.is_suspension() => {
                match self
                    .suspend_child_run(child_run_id, &err, child_ctx.total_cost_usd, total_duration)
                    .await
                {
                    Ok(()) => {
                        info!(
                            parent_run_id = %self.run_id,
                            child_run_id = %child_run_id,
                            cause = %err.suspension_leaf(),
                            "child run suspended"
                        );
                        Err(EngineError::ChildSuspended {
                            run_id: child_run_id,
                            cause: Box::new(err),
                        })
                    }
                    Err(store_err) => {
                        self.fail_child_run(
                            child_run_id,
                            RunStatus::Failed,
                            &store_err,
                            child_ctx.total_cost_usd,
                            total_duration,
                            child_ctx.output().cloned(),
                        )
                        .await;
                        Err(store_err)
                    }
                }
            }
            Err(err) => {
                // The engine cancels top-level runs stopped by a guardrail.
                let status = if matches!(
                    err,
                    EngineError::RunBudgetExceeded { .. } | EngineError::WorkflowGuardRejected(_)
                ) {
                    RunStatus::Cancelled
                } else {
                    RunStatus::Failed
                };
                self.fail_child_run(
                    child_run_id,
                    status,
                    &err,
                    child_ctx.total_cost_usd,
                    total_duration,
                    child_ctx.output().cloned(),
                )
                .await;
                if config.allow_failure {
                    return Ok(ChildOutcome::Finished(
                        SubWorkflowOutput::new(
                            child_run_id,
                            &config.workflow_name,
                            status,
                            child_ctx.total_cost_usd,
                            total_duration,
                        )
                        .with_output(child_ctx.output().cloned())
                        .with_error(err.to_string()),
                        true,
                    ));
                }
                Err(err)
            }
        }
    }

    /// Create the child run of a sub-workflow step and return its id.
    ///
    /// The child inherits the parent labels and author, and is linked to its
    /// parent and to the root of the chain by two labels, so a suspended child
    /// can be found and resumed like a top-level run.
    async fn create_child_run(&self, config: &WorkflowStepConfig) -> Result<Uuid, EngineError> {
        // Whoever triggered the parent workflow is accountable for its children.
        let parent = self.store.get_run(self.run_id).await?;
        let (mut labels, parent_author) =
            parent.map(|r| (r.labels, r.created_by)).unwrap_or_default();
        // Overwritten, never inherited: a grand-child must point at its own
        // parent, not at its grand-parent.
        labels.insert(PARENT_RUN_ID_LABEL.to_string(), self.run_id.to_string());
        labels.insert(LABEL_ROOT_RUN_ID.to_string(), self.root_run_id.to_string());

        let child_run = self
            .store
            .create_run(NewRun {
                workflow_name: config.workflow_name.clone(),
                trigger: TriggerKind::Workflow,
                payload: config.payload.clone(),
                max_retries: 0,
                handler_version: None,
                labels,
                scheduled_at: None,
                created_by: parent_author,
                idempotency_key: None,
                concurrency_key: config.concurrency_key.clone(),
                // A child runs inside its parent's slot: it never consumes a
                // concurrency group slot of its own.
                concurrency_limits: Vec::new(),
                // The child shares the parent's cap; it does not get its own budget.
                max_cost_usd: self.max_cost_usd,
            })
            .await?
            .into_run();

        info!(
            parent_run_id = %self.run_id,
            child_run_id = %child_run.id,
            workflow = %config.workflow_name,
            "child run created"
        );
        Ok(child_run.id)
    }

    /// Persist the suspension of a child run, with no event: the root run
    /// publishes the suspension once the whole chain is suspended.
    ///
    /// A direct suspension is persisted like a top-level run's (a delay or a
    /// signal deadline arms `scheduled_at`). A child suspended because of its
    /// own child gets no `scheduled_at`: only the deepest run owns the
    /// wake-up, so the chain is never resumed twice.
    async fn suspend_child_run(
        &self,
        child_run_id: Uuid,
        err: &EngineError,
        cost_usd: Decimal,
        duration_ms: u64,
    ) -> Result<(), EngineError> {
        let totals = RunUpdate {
            cost_usd: Some(cost_usd),
            duration_ms: Some(duration_ms),
            ..RunUpdate::default()
        };

        let update = match err {
            EngineError::DelaySleeping { wake_at, .. } => RunUpdate {
                status: Some(RunStatus::Sleeping),
                scheduled_at: Some(*wake_at),
                ..totals
            },
            EngineError::SignalWaiting {
                step_id,
                deadline_at,
                ..
            } => {
                // Atomic with the step lock, like a top-level run.
                self.store
                    .suspend_run_on_signal(child_run_id, *step_id, *deadline_at)
                    .await?;
                totals
            }
            EngineError::ChildSuspended { cause, .. } => RunUpdate {
                status: Some(cause.suspension_status()),
                ..totals
            },
            _ => RunUpdate {
                status: Some(RunStatus::AwaitingApproval),
                ..totals
            },
        };

        self.store.update_run(child_run_id, update).await?;
        Ok(())
    }

    /// Mark a child run failed after its handler (or its suspension) failed.
    ///
    /// Best effort: the original error is what the parent reports.
    async fn fail_child_run(
        &self,
        child_run_id: Uuid,
        status: RunStatus,
        err: &EngineError,
        cost_usd: Decimal,
        duration_ms: u64,
        output: Option<Value>,
    ) {
        if let Err(store_err) = self
            .store
            .update_run(
                child_run_id,
                RunUpdate {
                    status: Some(status),
                    error: Some(err.to_string()),
                    cost_usd: Some(cost_usd),
                    duration_ms: Some(duration_ms),
                    completed_at: Some(Utc::now()),
                    output,
                    ..RunUpdate::default()
                },
            )
            .await
        {
            error!(
                child_run_id = %child_run_id,
                store_error = %store_err,
                "failed to persist child run failure"
            );
        }
    }
}
