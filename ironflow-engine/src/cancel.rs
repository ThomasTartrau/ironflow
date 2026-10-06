//! Cancellation of a run and of the sub-workflow runs below it.
//!
//! A child run executes inside its parent's execution: once the parent stops
//! (cancelled, failed, retried, abandoned by its worker), nothing drives the
//! child any more. [`Engine::cancel_descendants`] closes those children so
//! none stays non-terminal forever, holding its concurrency key.
//! [`Engine::cancel_run`] cancels a run with its descendants, and wakes the
//! root of a suspended chain whose child it cancelled so the root observes it.

use std::sync::Arc;

use chrono::Utc;
use tokio::spawn;
use tracing::{error, info, warn};
use uuid::Uuid;

use ironflow_store::error::StoreError;
use ironflow_store::models::{Run, RunStatus, RunUpdate};

use crate::engine::{Engine, ExecutionMode, chain_root};
use crate::error::EngineError;
use crate::notify::{Event, RunStatusChangedEvent};

/// Error recorded on the steps left open by a cancelled run.
///
/// # Examples
///
/// ```
/// use ironflow_engine::cancel::RUN_CANCELLED_ERROR;
///
/// assert_eq!(RUN_CANCELLED_ERROR, "run cancelled");
/// ```
pub const RUN_CANCELLED_ERROR: &str = "run cancelled";

/// Outcome of [`Engine::cancel_run`].
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
/// let cancellation = engine.cancel_run(run_id).await?;
/// println!(
///     "run {} cancelled with {} sub-runs",
///     cancellation.run.id,
///     cancellation.cancelled_descendants.len()
/// );
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct RunCancellation {
    /// The cancelled run, as stored after the cancellation.
    pub run: Run,
    /// The sub-workflow runs below it that this call cancelled, oldest first.
    pub cancelled_descendants: Vec<Uuid>,
}

impl Engine {
    /// Cancel a run and every non-terminal sub-workflow run below it.
    ///
    /// The run moves to `Cancelled`, its open steps are closed with
    /// [`RUN_CANCELLED_ERROR`] and [`Event::RunStatusChanged`] is published.
    /// Its descendants are then cancelled by
    /// [`cancel_descendants`](Self::cancel_descendants), which releases their
    /// concurrency keys.
    ///
    /// When the run is a child whose root waits suspended with it
    /// (`AwaitingApproval` or `Sleeping`), the root is woken so its open
    /// `Workflow` step observes the cancellation: under
    /// [`ExecutionMode::Local`] it resumes in a background task, under
    /// [`ExecutionMode::Workers`] it goes back to `Pending` for a worker. The
    /// parent then fails, unless its step tolerates the failure with
    /// `allow_failure`.
    ///
    /// Cancelling a run already `Cancelled` is accepted: it publishes nothing
    /// for the run itself and cancels whatever descendant is still active.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::Store`] with [`StoreError::RunNotFound`] for an
    /// unknown run, with [`StoreError::InvalidTransition`] for a run that
    /// already finished otherwise (`Completed`, `Failed`, `Warning`), and with
    /// the store error when the cancellation cannot be persisted.
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
    /// let cancellation = engine.cancel_run(run_id).await?;
    /// assert!(cancellation.run.status.state.is_terminal());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn cancel_run(
        self: &Arc<Self>,
        run_id: Uuid,
    ) -> Result<RunCancellation, EngineError> {
        let run = self.load_run(run_id).await?;
        let from = run.status.state;
        if !from.can_transition_to(&RunStatus::Cancelled) {
            return Err(EngineError::Store(StoreError::InvalidTransition {
                from,
                to: RunStatus::Cancelled,
            }));
        }

        if from != RunStatus::Cancelled {
            self.store()
                .update_run(
                    run_id,
                    RunUpdate {
                        status: Some(RunStatus::Cancelled),
                        completed_at: Some(Utc::now()),
                        ..RunUpdate::default()
                    },
                )
                .await?;
            self.publish_cancelled(&run, None);
            info!(run_id = %run_id, from = %from, "run cancelled");
        }
        self.fail_orphaned_steps(run_id, RUN_CANCELLED_ERROR)
            .await?;

        let reason = format!("ancestor run {run_id} cancelled");
        let cancelled_descendants = self.cancel_descendants(run_id, &reason).await?;

        self.wake_root_of_cancelled_child(&run).await;

        Ok(RunCancellation {
            run: self.load_run(run_id).await?,
            cancelled_descendants,
        })
    }

    /// Cancel every non-terminal sub-workflow run below `run_id`, at any depth.
    ///
    /// Each descendant moves to `Cancelled` with `reason` as its error, its
    /// open steps are closed with the same reason, and
    /// [`Event::RunStatusChanged`] is published for it. A terminal descendant
    /// releases its concurrency key. A descendant that finished on its own in
    /// the meantime is left as it is. Returns the ids of the runs cancelled,
    /// oldest first: an empty list when nothing was left to cancel.
    ///
    /// Called when a run stops for good or ends an attempt
    /// ([`fail_or_schedule_retry`](Self::fail_or_schedule_retry)), when the
    /// reaper fails a run whose worker died, and by
    /// [`cancel_run`](Self::cancel_run).
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::Store`] when the descendants cannot be listed or
    /// one of them cannot be updated.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::engine::Engine;
    /// use ironflow_engine::error::EngineError;
    /// use uuid::Uuid;
    ///
    /// # async fn example(engine: &Engine, run_id: Uuid) -> Result<(), EngineError> {
    /// let cancelled = engine
    ///     .cancel_descendants(run_id, "parent run stopped")
    ///     .await?;
    /// println!("{} sub-runs cancelled", cancelled.len());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn cancel_descendants(
        &self,
        run_id: Uuid,
        reason: &str,
    ) -> Result<Vec<Uuid>, EngineError> {
        let descendants = self.store().list_active_descendants(run_id).await?;
        let mut cancelled = Vec::with_capacity(descendants.len());

        for descendant in descendants {
            let update = RunUpdate {
                status: Some(RunStatus::Cancelled),
                error: Some(reason.to_string()),
                completed_at: Some(Utc::now()),
                ..RunUpdate::default()
            };
            match self.store().update_run(descendant.id, update).await {
                Ok(()) => {}
                // Finished on its own since it was listed: nothing to cancel.
                Err(StoreError::InvalidTransition { from, .. }) => {
                    info!(run_id = %descendant.id, status = %from, "descendant already finished");
                    continue;
                }
                Err(err) => return Err(err.into()),
            }
            self.fail_orphaned_steps(descendant.id, reason).await?;
            self.publish_cancelled(&descendant, Some(reason));
            cancelled.push(descendant.id);
        }

        if !cancelled.is_empty() {
            info!(
                run_id = %run_id,
                count = cancelled.len(),
                reason = %reason,
                "descendant runs cancelled"
            );
        }
        Ok(cancelled)
    }

    /// Close the descendants of a run whose attempt just ended, logging any
    /// failure instead of returning it: the run's own status is already
    /// persisted and must not be reported as unsaved.
    pub(crate) async fn cancel_descendants_of_stopped_run(&self, run_id: Uuid, error: &str) {
        let reason = format!("parent run {run_id} stopped: {error}");
        if let Err(err) = self.cancel_descendants(run_id, &reason).await {
            error!(run_id = %run_id, error = %err, "failed to cancel the descendants of a stopped run");
        }
    }

    /// Wake the root of `run`'s chain when it waits suspended with it.
    ///
    /// Best effort: the cancellation is already persisted, so a failure is
    /// logged and the root is left for an operator.
    async fn wake_root_of_cancelled_child(self: &Arc<Self>, run: &Run) {
        let Some(root_id) = chain_root(run) else {
            return;
        };
        let root = match self.store().get_run(root_id).await {
            Ok(Some(root)) => root,
            Ok(None) => return,
            Err(err) => {
                warn!(run_id = %run.id, root_run_id = %root_id, error = %err, "cannot read the root of a cancelled child");
                return;
            }
        };

        // A paused root is not woken: it resumes as an active run, so its
        // replay observes the cancelled child once an operator resumes it.
        if root.status.state == RunStatus::Paused {
            if let Err(err) = self.requeue_paused_root(run).await {
                warn!(run_id = %run.id, root_run_id = %root_id, error = %err, "cannot requeue the paused root of a cancelled child");
            }
            return;
        }

        let woken = match (root.status.state, self.execution_mode()) {
            (RunStatus::AwaitingApproval | RunStatus::Sleeping, ExecutionMode::Workers) => {
                self.store()
                    .update_run_status(root_id, RunStatus::Pending)
                    .await
            }
            (RunStatus::AwaitingApproval, ExecutionMode::Local) => {
                self.store()
                    .update_run_status(root_id, RunStatus::Running)
                    .await
            }
            (RunStatus::Sleeping, ExecutionMode::Local) => {
                match self
                    .store()
                    .update_run_status(root_id, RunStatus::Pending)
                    .await
                {
                    Ok(()) => {
                        self.store()
                            .update_run_status(root_id, RunStatus::Running)
                            .await
                    }
                    Err(err) => Err(err),
                }
            }
            // Running (the child runs inline), queued or finished: nothing waits.
            _ => return,
        };
        if let Err(err) = woken {
            warn!(run_id = %run.id, root_run_id = %root_id, error = %err, "cannot wake the root of a cancelled child");
            return;
        }

        info!(run_id = %run.id, root_run_id = %root_id, "root run woken to observe its cancelled child");
        if self.execution_mode() == ExecutionMode::Local {
            let engine = Arc::clone(self);
            spawn(async move {
                if let Err(err) = engine.resume_run(root_id).await {
                    error!(root_run_id = %root_id, error = %err, "root run stopped after its child was cancelled");
                }
            });
        }
    }

    /// Publish the move of `run` (as loaded before the update) to `Cancelled`.
    fn publish_cancelled(&self, run: &Run, error: Option<&str>) {
        self.event_publisher()
            .publish(Event::RunStatusChanged(RunStatusChangedEvent {
                run_id: run.id,
                workflow_name: run.workflow_name.clone(),
                from: run.status.state,
                to: RunStatus::Cancelled,
                error: error.map(str::to_string),
                cost_usd: run.cost_usd,
                duration_ms: run.duration_ms,
                labels: run.labels.clone(),
                at: Utc::now(),
            }));
    }

    pub(crate) async fn load_run(&self, run_id: Uuid) -> Result<Run, EngineError> {
        self.store()
            .get_run(run_id)
            .await?
            .ok_or(EngineError::Store(StoreError::RunNotFound(run_id)))
    }
}
