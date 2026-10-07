//! Operator pause of a run, or of a whole workflow.
//!
//! [`Engine::pause_run`] holds a root run, with every sub-workflow run below
//! it, in `Paused` until [`Engine::resume_paused_run`] puts each one back in
//! the state it was paused from, or [`Engine::cancel_run`] stops it. A run
//! executing when it is paused has its step in flight interrupted with
//! [`STEP_INTERRUPTED_ERROR`](ironflow_store::store::STEP_INTERRUPTED_ERROR),
//! no further step is started, and the resume replays the run: finished steps
//! are skipped and the interrupted step is executed again.
//!
//! [`Engine::pause_workflow`] holds the queued runs of a workflow instead:
//! they are created as usual but no worker picks them up until
//! [`Engine::resume_workflow`].

use std::sync::Arc;

use chrono::Utc;
use tracing::{debug, info};
use uuid::Uuid;

use ironflow_store::error::StoreError;
use ironflow_store::models::{Run, RunStatus, RunUpdate, WorkflowPause};

use crate::engine::{Engine, ExecutionMode, chain_root};
use crate::error::EngineError;
use crate::notify::{Event, RunStatusChangedEvent};

/// Outcome of [`Engine::pause_run`].
///
/// # Examples
///
/// ```no_run
/// use std::sync::Arc;
/// use ironflow_engine::engine::Engine;
/// use ironflow_engine::error::EngineError;
/// use uuid::Uuid;
///
/// # async fn example(engine: Arc<Engine>, run_id: Uuid) -> Result<(), EngineError> {
/// let pause = engine.pause_run(run_id).await?;
/// println!(
///     "run {} paused with {} sub-runs",
///     pause.run.id,
///     pause.paused_descendants.len()
/// );
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct RunPause {
    /// The paused run, as stored after the pause.
    pub run: Run,
    /// The sub-workflow runs below it that this call paused, oldest first.
    pub paused_descendants: Vec<Uuid>,
}

/// Outcome of [`Engine::resume_paused_run`].
///
/// # Examples
///
/// ```no_run
/// use std::sync::Arc;
/// use ironflow_engine::engine::Engine;
/// use ironflow_engine::error::EngineError;
/// use uuid::Uuid;
///
/// # async fn example(engine: Arc<Engine>, run_id: Uuid) -> Result<(), EngineError> {
/// let resume = engine.resume_paused_run(run_id).await?;
/// println!("run {} is {}", resume.run.id, resume.run.status.state);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct RunResume {
    /// The resumed run, as stored after the resume.
    pub run: Run,
    /// The sub-workflow runs below it that this call resumed, oldest first.
    pub resumed_descendants: Vec<Uuid>,
}

impl Engine {
    /// Pause a root run and every active sub-workflow run below it.
    ///
    /// Each run moves to `Paused` and records the state it was paused from in
    /// [`Run::resume_status`]. A run that was executing has its running steps
    /// marked `Failed` with
    /// [`STEP_INTERRUPTED_ERROR`](ironflow_store::store::STEP_INTERRUPTED_ERROR),
    /// so the resume executes them again. A run held by a worker loses its lease: the worker drops
    /// the execution at its next renewal, and the reaper never touches a
    /// paused run. A run executing in-process stops before its next step.
    /// [`Event::RunStatusChanged`] is published for every run paused.
    ///
    /// While the run is paused, an approval, a human input or a signal it
    /// waits for can still be resolved: the decision is recorded and only
    /// changes the state the run resumes to. The SLA deadline of an approval
    /// gate keeps running: the [`ApprovalEscalator`](crate::escalation::ApprovalEscalator)
    /// applies its policy during the pause, with the same effect as a human
    /// decision.
    ///
    /// # Errors
    ///
    /// - [`EngineError::Store`] with [`StoreError::RunNotFound`] for an
    ///   unknown run.
    /// - [`EngineError::ChildRunNotPausable`] for a sub-workflow run: pause
    ///   its root run instead.
    /// - [`EngineError::Store`] with [`StoreError::InvalidTransition`] for a
    ///   run already paused or finished.
    /// - [`EngineError::Store`] when the pause cannot be persisted.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::sync::Arc;
    /// use ironflow_engine::engine::Engine;
    /// use ironflow_engine::error::EngineError;
    /// use ironflow_store::models::RunStatus;
    /// use uuid::Uuid;
    ///
    /// # async fn example(engine: Arc<Engine>, run_id: Uuid) -> Result<(), EngineError> {
    /// let pause = engine.pause_run(run_id).await?;
    /// assert_eq!(pause.run.status.state, RunStatus::Paused);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn pause_run(&self, run_id: Uuid) -> Result<RunPause, EngineError> {
        let run = self.load_pausable_root(run_id).await?;
        let from = run.status.state;
        if !from.can_transition_to(&RunStatus::Paused) {
            return Err(EngineError::Store(StoreError::InvalidTransition {
                from,
                to: RunStatus::Paused,
            }));
        }

        // The root first: an execution in flight checks its own run before
        // every step, so it stops as soon as possible.
        self.store()
            .update_run_status(run_id, RunStatus::Paused)
            .await?;
        if from == RunStatus::Running {
            self.interrupt_running_steps(run_id).await?;
        }
        self.publish_transition(&run, RunStatus::Paused);
        info!(run_id = %run_id, from = %from, "run paused");

        let mut paused_descendants = Vec::new();
        for descendant in self.store().list_active_descendants(run_id).await? {
            if !descendant
                .status
                .state
                .can_transition_to(&RunStatus::Paused)
            {
                continue;
            }
            match self
                .store()
                .update_run_status(descendant.id, RunStatus::Paused)
                .await
            {
                Ok(()) => {}
                // Finished or paused since it was listed: nothing to pause.
                Err(StoreError::InvalidTransition { .. }) => {
                    debug!(
                        run_id = %descendant.id,
                        "descendant run no longer pausable, skipped"
                    );
                    continue;
                }
                Err(err) => return Err(err.into()),
            }
            if descendant.status.state == RunStatus::Running {
                self.interrupt_running_steps(descendant.id).await?;
            }
            self.publish_transition(&descendant, RunStatus::Paused);
            paused_descendants.push(descendant.id);
        }

        if !paused_descendants.is_empty() {
            info!(
                run_id = %run_id,
                count = paused_descendants.len(),
                "descendant runs paused"
            );
        }

        Ok(RunPause {
            run: self.load_run(run_id).await?,
            paused_descendants,
        })
    }

    /// Resume a paused root run and the sub-workflow runs paused with it.
    ///
    /// Each run goes back to the state recorded in [`Run::resume_status`]:
    /// a run paused while waiting (`Pending`, `Retrying`, `Sleeping`,
    /// `AwaitingApproval`) waits again, and a run whose approval, human input
    /// or signal was resolved during the pause is queued. A root that was
    /// executing is queued with its interrupted steps, which are executed
    /// again; finished steps are replayed. A sleeping root whose deadline
    /// passed during the pause is queued at once; a sleeping sub-workflow run
    /// in the same case is woken by the [`RunWaker`](crate::wake::RunWaker)
    /// on its next tick.
    ///
    /// Under [`ExecutionMode::Local`] a queued root is resumed in a background
    /// task, and so is a queued sub-workflow run whose root still waits.
    /// [`Event::RunStatusChanged`] is published for every run resumed.
    ///
    /// Under [`ExecutionMode::Local`], an execution still inside its step when
    /// the run is resumed is not doubled: the run goes back to `Running` and
    /// that execution carries on, and the background task waits for it to end
    /// before deciding whether anything is left to restart.
    ///
    /// # Errors
    ///
    /// - [`EngineError::Store`] with [`StoreError::RunNotFound`] for an
    ///   unknown run.
    /// - [`EngineError::ChildRunNotPausable`] for a sub-workflow run: resume
    ///   its root run instead.
    /// - [`EngineError::Store`] with [`StoreError::InvalidTransition`] for a
    ///   run that is not paused.
    /// - [`EngineError::Store`] when the resume cannot be persisted.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::sync::Arc;
    /// use ironflow_engine::engine::Engine;
    /// use ironflow_engine::error::EngineError;
    /// use ironflow_store::models::RunStatus;
    /// use uuid::Uuid;
    ///
    /// # async fn example(engine: Arc<Engine>, run_id: Uuid) -> Result<(), EngineError> {
    /// let resume = engine.resume_paused_run(run_id).await?;
    /// assert_ne!(resume.run.status.state, RunStatus::Paused);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn resume_paused_run(
        self: &Arc<Self>,
        run_id: Uuid,
    ) -> Result<RunResume, EngineError> {
        let run = self.load_pausable_root(run_id).await?;
        if run.status.state != RunStatus::Paused {
            return Err(EngineError::Store(StoreError::InvalidTransition {
                from: run.status.state,
                to: RunStatus::Pending,
            }));
        }

        // The descendants first: the root re-enters them as soon as it runs.
        let mut resumed_descendants = Vec::new();
        let mut queued_descendants = Vec::new();
        for descendant in self.store().list_active_descendants(run_id).await? {
            if descendant.status.state != RunStatus::Paused {
                continue;
            }
            // A child that was executing stays `Running`: its root's replay
            // re-enters it and executes its interrupted steps again.
            let target = descendant.resume_status.unwrap_or(RunStatus::Pending);
            self.store()
                .update_run_status(descendant.id, target)
                .await?;
            self.publish_transition(&descendant, target);
            if target == RunStatus::Pending {
                queued_descendants.push(descendant.id);
            }
            resumed_descendants.push(descendant.id);
        }

        let target = match run.resume_status {
            // Stopped by the pause in the middle of its execution: queued
            // again, like a run whose worker lost its lease.
            Some(RunStatus::Running) | None => {
                self.interrupt_running_steps(run_id).await?;
                RunStatus::Pending
            }
            // The deadline passed during the pause: due now, like a run the
            // waker would have claimed. A past `scheduled_at` does not hold
            // back the pick.
            Some(RunStatus::Sleeping) if run.scheduled_at.is_some_and(|at| at <= Utc::now()) => {
                RunStatus::Pending
            }
            Some(status) => status,
        };
        self.store().update_run_status(run_id, target).await?;
        self.publish_transition(&run, target);
        info!(run_id = %run_id, to = %target, "run resumed");

        if self.execution_mode() == ExecutionMode::Local {
            if target == RunStatus::Pending {
                self.continue_in_flight_execution(run_id).await?;
                self.spawn_local_resume(run_id);
            } else {
                // The root waits on its chain: the queued child resumes it.
                for child_id in queued_descendants {
                    self.continue_in_flight_execution(child_id).await?;
                    self.spawn_local_resume(child_id);
                }
            }
        }

        Ok(RunResume {
            run: self.load_run(run_id).await?,
            resumed_descendants,
        })
    }

    /// Hand a run just queued to `Pending` back to the execution still
    /// running it in this process, if any.
    ///
    /// That execution does not see the pause at a step boundary once the run
    /// left `Paused`, so the run must be `Running` for it to carry on as a
    /// legitimate run.
    async fn continue_in_flight_execution(&self, run_id: Uuid) -> Result<(), EngineError> {
        if self.is_executing(run_id) {
            self.store()
                .update_run_status(run_id, RunStatus::Running)
                .await?;
            debug!(run_id = %run_id, "execution still in flight, run continues");
        }
        Ok(())
    }

    /// Pause a registered workflow: its queued runs are no longer picked up.
    ///
    /// Runs keep being created; workers skip them until
    /// [`resume_workflow`](Self::resume_workflow). Runs already executing
    /// are not affected: pause them with [`pause_run`](Self::pause_run).
    /// Pausing a paused workflow returns the pause already recorded.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::InvalidWorkflow`] when no handler is registered
    /// under `workflow_name`, and [`EngineError::Store`] when the pause cannot
    /// be persisted.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::engine::Engine;
    /// use ironflow_engine::error::EngineError;
    ///
    /// # async fn example(engine: &Engine) -> Result<(), EngineError> {
    /// let pause = engine.pause_workflow("deploy", None).await?;
    /// println!("deploy paused at {}", pause.paused_at);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn pause_workflow(
        &self,
        workflow_name: &str,
        paused_by: Option<Uuid>,
    ) -> Result<WorkflowPause, EngineError> {
        self.require_handler(workflow_name)?;
        let pause = self
            .store()
            .pause_workflow(workflow_name, paused_by)
            .await?;
        info!(workflow = %workflow_name, "workflow paused");
        Ok(pause)
    }

    /// Resume a paused workflow so its queued runs are picked up again.
    ///
    /// Returns `true` when the workflow was paused, `false` when it was not:
    /// resuming is idempotent.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::InvalidWorkflow`] when no handler is registered
    /// under `workflow_name`, and [`EngineError::Store`] when the resume
    /// cannot be persisted.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::engine::Engine;
    /// use ironflow_engine::error::EngineError;
    ///
    /// # async fn example(engine: &Engine) -> Result<(), EngineError> {
    /// if engine.resume_workflow("deploy").await? {
    ///     println!("deploy resumed");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn resume_workflow(&self, workflow_name: &str) -> Result<bool, EngineError> {
        self.require_handler(workflow_name)?;
        let was_paused = self.store().resume_workflow(workflow_name).await?;
        if was_paused {
            info!(workflow = %workflow_name, "workflow resumed");
        }
        Ok(was_paused)
    }

    fn require_handler(&self, workflow_name: &str) -> Result<(), EngineError> {
        match self.get_handler(workflow_name) {
            Some(_) => Ok(()),
            None => Err(EngineError::InvalidWorkflow(format!(
                "no handler registered for workflow '{workflow_name}'"
            ))),
        }
    }

    /// Make the paused root of `run`'s chain resume to `Pending` when it
    /// waits suspended with its child, so that its replay observes what
    /// happened to the child (cancelled, rejected) once an operator resumes
    /// it.
    pub(crate) async fn requeue_paused_root(&self, run: &Run) -> Result<(), EngineError> {
        let Some(root_id) = chain_root(run) else {
            return Ok(());
        };
        let root = self.load_run(root_id).await?;
        let suspended = matches!(
            root.resume_status,
            Some(RunStatus::AwaitingApproval | RunStatus::Sleeping)
        );
        if root.status.state != RunStatus::Paused || !suspended {
            return Ok(());
        }

        let update = RunUpdate {
            resume_status: Some(RunStatus::Pending),
            ..RunUpdate::default()
        };
        self.store().update_run(root_id, update).await?;
        info!(run_id = %run.id, root_run_id = %root_id, "paused root run will resume to observe its child");
        Ok(())
    }

    /// Load `run_id`, refusing a sub-workflow run.
    async fn load_pausable_root(&self, run_id: Uuid) -> Result<Run, EngineError> {
        let run = self.load_run(run_id).await?;
        match chain_root(&run) {
            Some(root_run_id) => Err(EngineError::ChildRunNotPausable {
                run_id,
                root_run_id,
            }),
            None => Ok(run),
        }
    }

    /// Publish the move of `run` (as loaded before the update) to `to`.
    fn publish_transition(&self, run: &Run, to: RunStatus) {
        self.event_publisher()
            .publish(Event::RunStatusChanged(RunStatusChangedEvent {
                run_id: run.id,
                workflow_name: run.workflow_name.clone(),
                from: run.status.state,
                to,
                error: None,
                cost_usd: run.cost_usd,
                duration_ms: run.duration_ms,
                labels: run.labels.clone(),
                at: Utc::now(),
            }));
    }
}
