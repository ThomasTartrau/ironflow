//! Resuming the root run of a sub-workflow chain from one of its children.
//!
//! A chain executes as one execution of its root run. A child that suspends
//! owns what resumes it (an approval, an answer, a delay, a signal); resuming
//! the child resumes the root, which re-enters the child through its open
//! `Workflow` step. The children of a
//! [`workflows_with`](crate::context::WorkflowContext::workflows_with) wave
//! suspend side by side, so a child may come back while a sibling already
//! resumed the root: it waits for the root to stop instead of failing, and the
//! root's status transition decides which resume starts it.

use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio::time::sleep;
use tracing::{error, info};
use uuid::Uuid;

use ironflow_store::error::StoreError;
use ironflow_store::models::{Run, RunStatus};

use crate::engine::Engine;
use crate::error::EngineError;
use crate::notify::{Event, SignalAwaitedEvent};

/// How often a child waiting for its busy root reads it again.
const BUSY_ROOT_POLL: Duration = Duration::from_millis(100);

/// What resuming a child did with the root of its chain.
pub(crate) enum ChainClaim {
    /// The root moved to `Running` for this child: resume it.
    Claimed,
    /// The root re-entered the child while it ran: nothing is left to resume.
    /// Holds the root as it stands.
    Settled(Box<Run>),
}

impl Engine {
    /// Move the root of `child_run_id`'s chain to `Running` so the child can
    /// be resumed through it.
    ///
    /// A root already running was resumed by a sibling of the child: the call
    /// waits until it stops. If the root re-entered the child meanwhile, the
    /// child no longer waits and nothing is resumed. When two children come
    /// back at once, the root's transition lets one of them through and the
    /// other waits.
    ///
    /// With `lease`, the root takes the worker lease of the child, atomically
    /// with its move to `Running`.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::InvalidWorkflow`] when the root stopped for good
    /// (finished, paused, retrying) while the child still waits: the child is
    /// failed. Returns [`EngineError::Store`] when a run cannot be read or
    /// written.
    pub(crate) async fn claim_chain_root(
        &self,
        child_run_id: Uuid,
        root_run_id: Uuid,
        lease: Option<&(String, DateTime<Utc>)>,
    ) -> Result<ChainClaim, EngineError> {
        loop {
            let root = self.existing_run(root_run_id).await?;
            let claimed = match root.status.state {
                status @ (RunStatus::AwaitingApproval
                | RunStatus::Pending
                | RunStatus::Sleeping) => {
                    self.move_suspended_root(root_run_id, status, lease).await?
                }
                // Resumed by a sibling of this child: wait for it to stop.
                RunStatus::Running => false,
                other => {
                    if !self.child_awaits_root(child_run_id).await? {
                        return Ok(ChainClaim::Settled(Box::new(root)));
                    }
                    let reason = format!(
                        "cannot resume child run {child_run_id}: root run {root_run_id} is {other}"
                    );
                    if let Err(err) = self
                        .fail_or_schedule_retry(child_run_id, &reason, false, None, None)
                        .await
                    {
                        error!(
                            run_id = %child_run_id,
                            error = %err,
                            "failed to fail a child run whose root cannot resume"
                        );
                    }
                    return Err(EngineError::InvalidWorkflow(reason));
                }
            };
            if claimed {
                return Ok(ChainClaim::Claimed);
            }
            if !self.child_awaits_root(child_run_id).await? {
                info!(
                    run_id = %child_run_id,
                    root_run_id = %root_run_id,
                    "child run re-entered by its running root"
                );
                let root = self.existing_run(root_run_id).await?;
                return Ok(ChainClaim::Settled(Box::new(root)));
            }
            sleep(BUSY_ROOT_POLL).await;
        }
    }

    /// Move a suspended root to `Running`. `Ok(false)` when another resume
    /// moved it between the read of `status` and this write.
    async fn move_suspended_root(
        &self,
        root_run_id: Uuid,
        status: RunStatus,
        lease: Option<&(String, DateTime<Utc>)>,
    ) -> Result<bool, EngineError> {
        let moved = match status {
            RunStatus::Sleeping => {
                match self
                    .store()
                    .update_run_status(root_run_id, RunStatus::Pending)
                    .await
                {
                    Ok(()) => self.move_root_to_running(root_run_id, lease).await,
                    Err(err) => Err(err.into()),
                }
            }
            _ => self.move_root_to_running(root_run_id, lease).await,
        };
        match moved {
            Ok(()) => Ok(true),
            Err(EngineError::Store(StoreError::InvalidTransition { .. })) => Ok(false),
            Err(err) => Err(err),
        }
    }

    /// Whether the child still waits for its root to re-enter it: answered,
    /// approved or woken, and not executed since.
    async fn child_awaits_root(&self, child_run_id: Uuid) -> Result<bool, EngineError> {
        let child = self.existing_run(child_run_id).await?;
        Ok(matches!(
            child.status.state,
            RunStatus::Running | RunStatus::Pending
        ))
    }

    async fn existing_run(&self, run_id: Uuid) -> Result<Run, EngineError> {
        self.store()
            .get_run(run_id)
            .await?
            .ok_or(EngineError::Store(StoreError::RunNotFound(run_id)))
    }

    /// Publish what a suspended run of a chain waits for: an approval or a
    /// signal. A human input or a delay publishes no suspension event, like
    /// on a top-level run.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::Store`] when the approval step cannot be read.
    pub(crate) async fn publish_suspension(&self, leaf: &EngineError) -> Result<(), EngineError> {
        match leaf {
            EngineError::ApprovalRequired {
                run_id,
                step_id,
                message,
            } => {
                self.publish_approval_requested(*run_id, *step_id, message)
                    .await?;
            }
            EngineError::SignalWaiting {
                run_id,
                step_id,
                step_name,
                name,
                key,
                deadline_at,
            } => {
                self.event_publisher()
                    .publish(Event::SignalAwaited(SignalAwaitedEvent {
                        run_id: *run_id,
                        step_id: *step_id,
                        step_name: step_name.clone(),
                        name: name.clone(),
                        key: key.clone(),
                        deadline_at: *deadline_at,
                        at: Utc::now(),
                    }));
            }
            _ => {}
        }
        Ok(())
    }
}
