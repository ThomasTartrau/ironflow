//! A wave of sub-workflows of one handler, run concurrently.
//!
//! Every child gets a `Workflow` step at the same position. Steps and child
//! runs are created in list order, then the children execute concurrently in
//! the parent's task: each child future borrows the parent context, like a
//! single `workflow_with` does. The parent closes the steps in list order.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use futures_util::future::join_all;
use serde_json::to_value;
use uuid::Uuid;

use ironflow_store::models::{Step, StepKind, StepStatus};

use crate::config::{WorkflowOptions, WorkflowStepConfig};
use crate::context::WorkflowContext;
use crate::context::cost::SharedCost;
use crate::context::lifecycle::check_replay_identity;
use crate::error::EngineError;
use crate::executor::SubWorkflowOutcome;
use crate::guard::{SharedGuardState, WorkflowRejection};
use crate::handler::{TypedWorkflow, WorkflowHandler};
use crate::plan::{SharedPlanRecorder, lock_plan};

use super::child::ChildStart;
use super::child_run::ChildRun;
use super::{ChildResume, leaves_step_open, recorded_child_run_id};

/// Where a child of a wave stands before the wave runs.
enum WaveSlot {
    /// Completed in a previous execution: replayed as recorded.
    Replayed {
        step_id: Uuid,
        outcome: SubWorkflowOutcome,
    },
    /// Left open by a child that suspended: the step is reused.
    Open(Box<Step>),
    /// Recorded now, re-entering the child run of a step interrupted by a
    /// lost lease, if any.
    New(Option<Uuid>),
}

/// A child of a wave once its step is recorded.
enum WaveChild {
    Replayed {
        step_id: Uuid,
        outcome: SubWorkflowOutcome,
    },
    Started {
        step_id: Uuid,
        config: WorkflowStepConfig,
        start: Result<ChildStart, EngineError>,
    },
}

/// A child of a wave once it ran.
enum WaveResult {
    Replayed {
        step_id: Uuid,
        outcome: SubWorkflowOutcome,
    },
    Ran {
        step_id: Uuid,
        config: Box<WorkflowStepConfig>,
        run: ChildRun,
    },
}

/// Reject a wave in which two children share a step name.
///
/// The step name identifies a child of the wave in the run timeline and on
/// resume, where two children with one name would re-enter one child run.
fn reject_duplicate_step_names(wave: &[(String, WorkflowStepConfig)]) -> Result<(), EngineError> {
    let mut seen = HashSet::with_capacity(wave.len());
    for (name, _) in wave {
        if !seen.insert(name.as_str()) {
            return Err(EngineError::StepConfig(format!(
                "sub-workflow wave has two steps named {name:?}; give each child its own step name"
            )));
        }
    }
    Ok(())
}

impl WorkflowContext {
    /// Execute a wave of sub-workflows of one handler, concurrently.
    ///
    /// Each entry starts one child run of `handler` with its input and its
    /// [`WorkflowOptions`], as [`workflow_with`](Self::workflow_with) does:
    /// step name, labels, concurrency key, priority and `allow_failure` are
    /// set per child. Every child gets a `Workflow` step at the same
    /// position, and the outcomes come back in the order of `children`.
    ///
    /// The child runs are created in list order, then executed concurrently;
    /// the call returns once every child has finished or suspended. A
    /// concurrency key already held, by an earlier sibling or by any other
    /// active run, gives a [`SubWorkflowOutcome::Conflict`] for that entry,
    /// replayed as-is on resume.
    ///
    /// The cost cap of the root run holds for the whole tree: every agent step
    /// of every child checks it against what the root, the children and their
    /// descendants have spent, plus the budget their running steps hold. A
    /// wave never takes the root run past its cap; a child refused by the cap
    /// ends `Cancelled` with `RUN_BUDGET_EXCEEDED`. The workflow guard counts
    /// each child as one invocation.
    ///
    /// A child that suspends leaves its step open while its siblings run to
    /// the end; the parent then suspends with it. On resume, completed steps
    /// are replayed and suspended children are re-entered: no child run is
    /// created twice. Each suspended child publishes its own event (approval
    /// requested, signal awaited). An empty list records nothing.
    ///
    /// # Errors
    ///
    /// Before anything runs: [`EngineError::StepConfig`] when two entries share
    /// a step name, [`EngineError::Serialization`] if an input cannot be
    /// serialized, [`EngineError::InvalidWorkflow`] when `handler` is not
    /// registered, and [`EngineError::WorkflowGuardRejected`] when the guard
    /// refuses the wave (a wave that would cross `max_fan_out` is refused
    /// whole). Returns [`EngineError::ReplayDivergence`] when the step recorded
    /// under an entry's name has another kind.
    ///
    /// Once every child has settled and its step is recorded: the first child
    /// failure in list order that `allow_failure` does not tolerate, otherwise
    /// [`EngineError::ChildSuspended`] for the first suspended child in list
    /// order.
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
    /// struct DelegateInput {
    ///     agent: String,
    /// }
    ///
    /// struct Delegate;
    ///
    /// impl WorkflowHandler for Delegate {
    ///     fn name(&self) -> &str { "delegate" }
    ///     fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
    ///         Box::pin(async move { Ok(()) })
    ///     }
    /// }
    ///
    /// impl TypedWorkflow for Delegate {
    ///     type Input = DelegateInput;
    /// }
    ///
    /// # async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    /// let tour = 1;
    /// let wave = ["alice", "bob"]
    ///     .into_iter()
    ///     .map(|agent| {
    ///         let options = WorkflowOptions::new()
    ///             .step_name(format!("delegate-{tour}-{agent}"))
    ///             .label("agent", agent)
    ///             .allow_failure();
    ///         (DelegateInput { agent: agent.to_string() }, options)
    ///     })
    ///     .collect();
    /// for outcome in ctx.workflows_with(&Delegate, wave).await? {
    ///     if let SubWorkflowOutcome::Completed(child) = outcome {
    ///         println!("run {} ended {}", child.run_id(), child.status());
    ///     }
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn workflows_with<W: TypedWorkflow>(
        &mut self,
        handler: &W,
        children: Vec<(W::Input, WorkflowOptions)>,
    ) -> Result<Vec<SubWorkflowOutcome>, EngineError> {
        let mut wave = Vec::with_capacity(children.len());
        for (input, options) in children {
            wave.push(options.into_step_config(handler.name(), to_value(&input)?));
        }
        reject_duplicate_step_names(&wave)?;
        if wave.is_empty() {
            return Ok(Vec::new());
        }

        // Plan mode: the wave is one parallel group, every child expanded.
        if let Some(plan) = self.plan().cloned() {
            return self.plan_sub_workflow_wave(&plan, handler, wave).await;
        }
        self.run_sub_workflow_wave(handler, wave).await
    }

    /// Record, then run, a wave of `Workflow` steps at one position.
    async fn run_sub_workflow_wave(
        &mut self,
        handler: &dyn WorkflowHandler,
        wave: Vec<(String, WorkflowStepConfig)>,
    ) -> Result<Vec<SubWorkflowOutcome>, EngineError> {
        let position = self.position;

        // Replay first: a child whose step completed in a previous execution
        // returns its recorded outcome and is not counted by the guard again.
        let mut slots = Vec::with_capacity(wave.len());
        for (step_name, _) in &wave {
            slots.push(self.wave_slot(position, step_name)?);
        }
        let to_run = slots
            .iter()
            .filter(|slot| !matches!(slot, WaveSlot::Replayed { .. }))
            .count();
        if to_run == 0 {
            self.position += 1;
            let (step_ids, outcomes): (Vec<Uuid>, Vec<SubWorkflowOutcome>) = slots
                .into_iter()
                .filter_map(|slot| match slot {
                    WaveSlot::Replayed { step_id, outcome } => Some((step_id, outcome)),
                    WaveSlot::Open(_) | WaveSlot::New(_) => None,
                })
                .unzip();
            self.last_step_ids = step_ids;
            return Ok(outcomes);
        }

        let child_handler = self.resolve_child_handler(handler.name())?;
        // A child runs on the worker that runs its parent: refuse it here
        // rather than run it on a host missing what it requires.
        self.check_child_worker_tags(handler)?;
        // Every child of the wave is claimed at once, or none is.
        if let (Some(guard_config), Some(guard_state)) = (&self.guard_config, &self.guard_state) {
            let state = guard_state
                .lock()
                .map_err(|_| WorkflowRejection::GuardUnavailable)?;
            let count = u32::try_from(to_run).unwrap_or(u32::MAX);
            state.enter_wave(guard_config, handler.name(), count)?;
        }
        // Paused while it ran: no child is started, the run stops here.
        self.check_not_paused().await?;
        self.position += 1;

        // The children share one live total of the tree, so the cap holds for
        // the wave as a whole rather than for each child against a snapshot.
        let shared_cost = match &self.shared_cost {
            Some(shared) => shared.clone(),
            None => SharedCost::new(self.charged_cost_usd()),
        };

        // Steps and child runs are created in list order, so a key held by an
        // earlier sibling conflicts the same way on every execution.
        let mut children = Vec::with_capacity(slots.len());
        for ((step_name, config), slot) in wave.into_iter().zip(slots) {
            let (step_id, resume) = match slot {
                WaveSlot::Replayed { step_id, outcome } => {
                    children.push(WaveChild::Replayed { step_id, outcome });
                    continue;
                }
                WaveSlot::Open(step) => (
                    step.id,
                    recorded_child_run_id(&step).map(ChildResume::Suspended),
                ),
                WaveSlot::New(interrupted) => (
                    self.record_workflow_step(&step_name, &config, position)
                        .await?,
                    interrupted.map(ChildResume::Interrupted),
                ),
            };
            let start = match self.branch_guard(handler.name()) {
                Ok(guard_state) => {
                    self.prepare_child(
                        &config,
                        step_id,
                        resume,
                        child_handler.clone(),
                        guard_state,
                        Some(shared_cost.clone()),
                    )
                    .await
                }
                Err(err) => Err(err),
            };
            children.push(WaveChild::Started {
                step_id,
                config,
                start,
            });
        }

        let this = &*self;
        let results = join_all(children.into_iter().map(|child| this.run_wave_child(child))).await;

        // Steps are closed in list order: the first failure, otherwise the
        // first suspension, is what the parent reports.
        let mut outcomes = Vec::with_capacity(results.len());
        let mut step_ids = Vec::with_capacity(results.len());
        let mut failure = None;
        let mut suspension = None;
        let mut sibling_suspensions = Vec::new();
        for result in results {
            let (step_id, settled) = match result {
                WaveResult::Replayed { step_id, outcome } => (step_id, Ok(outcome)),
                WaveResult::Ran {
                    step_id,
                    config,
                    run,
                } => {
                    if matches!(&run.result, Err(err) if err.is_suspension()) {
                        sibling_suspensions.extend(run.sibling_suspensions);
                    }
                    let settled = self.settle_child_step(step_id, &config, run.result).await;
                    (step_id, settled)
                }
            };
            step_ids.push(step_id);
            match settled {
                Ok(outcome) => outcomes.push(outcome),
                Err(err) if leaves_step_open(&err) => {
                    if suspension.is_none() {
                        suspension = Some(err);
                    } else if err.is_suspension() {
                        sibling_suspensions.push(err);
                    }
                }
                Err(err) => {
                    if failure.is_none() {
                        failure = Some(err);
                    }
                }
            }
        }

        if let Some(err) = failure {
            return Err(err);
        }
        if let Some(err) = suspension {
            self.sibling_suspensions.extend(sibling_suspensions);
            return Err(err);
        }
        self.last_step_ids = step_ids;
        Ok(outcomes)
    }

    /// Where the child named `step_name` stands at the wave `position`.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::ReplayDivergence`] when the step recorded under
    /// that name is not a `Workflow` step.
    fn wave_slot(&mut self, position: u32, step_name: &str) -> Result<WaveSlot, EngineError> {
        let key = (position, step_name.to_string());
        if let Some(step) = self.replay_wave_steps.get(&key).cloned() {
            check_replay_identity(&step, position, step_name, &StepKind::Workflow)?;
            match step.status.state {
                StepStatus::Completed => {
                    let outcome = self.replay_recorded_child(&step)?;
                    return Ok(WaveSlot::Replayed {
                        step_id: step.id,
                        outcome,
                    });
                }
                StepStatus::Running => return Ok(WaveSlot::Open(Box::new(step))),
                _ => {}
            }
        }
        let interrupted = match self.interrupted_children.get(&key) {
            Some(step) => {
                check_replay_identity(step, position, step_name, &StepKind::Workflow)?;
                recorded_child_run_id(step)
            }
            None => None,
        };
        Ok(WaveSlot::New(interrupted))
    }

    /// A guard state of its own for a child running concurrently with its
    /// siblings, its invocation claimed with the wave: depth and call chain
    /// per branch, invocations and tokens shared by the whole run tree.
    fn branch_guard(&self, target: &str) -> Result<Option<SharedGuardState>, EngineError> {
        let Some(guard_state) = &self.guard_state else {
            return Ok(None);
        };
        let branch = guard_state
            .lock()
            .map_err(|_| WorkflowRejection::GuardUnavailable)?
            .branch_into(target);
        Ok(Some(Arc::new(Mutex::new(branch))))
    }

    /// Execute one child of a wave, unless it was replayed or settled at once.
    async fn run_wave_child(&self, child: WaveChild) -> WaveResult {
        match child {
            WaveChild::Replayed { step_id, outcome } => WaveResult::Replayed { step_id, outcome },
            WaveChild::Started {
                step_id,
                config,
                start,
            } => {
                let run = match start {
                    Ok(ChildStart::Run(prepared)) => self.run_child(&config, *prepared).await,
                    Ok(ChildStart::Settled(outcome)) => ChildRun::from(Ok(outcome)),
                    Err(err) => ChildRun::from(Err(err)),
                };
                WaveResult::Ran {
                    step_id,
                    config: Box::new(config),
                    run,
                }
            }
        }
    }

    /// Record a wave while planning: one parallel group, every child expanded
    /// after its own step when the depth limit allows it.
    async fn plan_sub_workflow_wave(
        &mut self,
        plan: &SharedPlanRecorder,
        handler: &dyn WorkflowHandler,
        wave: Vec<(String, WorkflowStepConfig)>,
    ) -> Result<Vec<SubWorkflowOutcome>, EngineError> {
        self.position += 1;
        {
            let mut recorder = lock_plan(plan);
            let group = recorder.next_group();
            for (step_name, _) in &wave {
                recorder.record_or_stop(
                    step_name,
                    StepKind::Workflow,
                    &self.workflow_name,
                    Some(group.clone()),
                )?;
            }
        }

        // The steps after the wave depend on where every child ended.
        let mut frontier = Vec::new();
        let mut outcomes = Vec::with_capacity(wave.len());
        for (step_name, config) in wave {
            lock_plan(plan).set_last(vec![step_name]);
            let planned = self
                .expand_planned_child(plan, handler, config.payload)
                .await;
            frontier.extend(lock_plan(plan).last().iter().cloned());
            outcomes.push(SubWorkflowOutcome::Completed(planned));
        }
        lock_plan(plan).set_last(frontier);
        Ok(outcomes)
    }
}
