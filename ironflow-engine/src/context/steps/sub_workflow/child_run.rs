//! Execution of a prepared child run, and how it is closed.
//!
//! [`run_child`](WorkflowContext::run_child) takes the parent by shared
//! reference only, so the children of a wave execute concurrently.

use std::mem::take;
use std::time::Instant;

use chrono::Utc;
use rust_decimal::Decimal;
use serde_json::Value;
use tracing::{error, info};
use uuid::Uuid;

use ironflow_store::models::{ProviderKind, RunStatus, RunUpdate};

use crate::config::WorkflowStepConfig;
use crate::context::WorkflowContext;
use crate::error::EngineError;
use crate::executor::SubWorkflowOutput;

use super::ChildOutcome;
use super::child::{PreparedChild, cancelled_child_outcome};

/// What executing the child of a `Workflow` step produced.
pub(super) struct ChildRun {
    pub(super) result: Result<ChildOutcome, EngineError>,
    /// Suspensions of the child's own wave children that `result` does not
    /// carry, passed up so that each one publishes its event.
    pub(super) sibling_suspensions: Vec<EngineError>,
}

impl From<Result<ChildOutcome, EngineError>> for ChildRun {
    fn from(result: Result<ChildOutcome, EngineError>) -> Self {
        Self {
            result,
            sibling_suspensions: Vec::new(),
        }
    }
}

impl WorkflowContext {
    /// Execute a prepared child run and close it.
    ///
    /// A child that suspends is left in its suspension status and
    /// [`EngineError::ChildSuspended`] is returned, with the suspensions of
    /// its own wave children it does not carry.
    pub(super) async fn run_child(
        &self,
        config: &WorkflowStepConfig,
        child: PreparedChild,
    ) -> ChildRun {
        let PreparedChild {
            run_id: child_run_id,
            handler,
            ctx: mut child_ctx,
            resumed,
        } = child;

        let run_start = Instant::now();

        // A re-entered child replays its completed steps and is served the
        // answer, signal or elapsed delay it was suspended on.
        let loaded = if resumed {
            child_ctx.load_replay_steps().await
        } else {
            Ok(())
        };
        let result = match loaded {
            Ok(()) => handler.execute(&mut child_ctx).await,
            Err(err) => Err(err),
        };
        let total_duration = child_ctx.carried_duration_ms + run_start.elapsed().as_millis() as u64;

        let result = self
            .close_child_run(config, child_run_id, &child_ctx, result, total_duration)
            .await;
        match &result {
            Err(err) if err.is_suspension() => ChildRun {
                result,
                sibling_suspensions: take(&mut child_ctx.sibling_suspensions),
            },
            _ => ChildRun::from(result),
        }
    }

    /// Persist how the handler of a child run ended and turn it into the
    /// outcome of the parent's step.
    async fn close_child_run(
        &self,
        config: &WorkflowStepConfig,
        child_run_id: Uuid,
        child_ctx: &WorkflowContext,
        result: Result<(), EngineError>,
        total_duration: u64,
    ) -> Result<ChildOutcome, EngineError> {
        let completed_at = Utc::now();

        // Cancelled while it ran: whatever the handler returned, the child
        // stays cancelled and the cancellation is its outcome.
        if let Some(child_run) = self.store.get_run(child_run_id).await? {
            match child_run.status.state {
                RunStatus::Cancelled => {
                    return cancelled_child_outcome(
                        config,
                        &child_run,
                        child_ctx.total_cost_usd,
                        total_duration,
                        child_ctx.output().cloned(),
                    );
                }
                // Paused with its root while it ran: the child is left as the
                // pause recorded it and the parent stops with it.
                RunStatus::Paused => {
                    info!(
                        parent_run_id = %self.run_id,
                        child_run_id = %child_run_id,
                        "child run paused"
                    );
                    return Err(EngineError::RunPaused {
                        run_id: child_run_id,
                    });
                }
                _ => {}
            }
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

                Ok(ChildOutcome::Finished(
                    SubWorkflowOutput::new(
                        child_run_id,
                        &config.workflow_name,
                        child_status,
                        child_ctx.total_cost_usd,
                        total_duration,
                    )
                    .with_output(child_ctx.output().cloned()),
                    child_ctx.has_allowed_failure,
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

    /// Persist the suspension of a child run, with no event: the root run
    /// publishes the suspension once the whole chain is suspended.
    ///
    /// A direct suspension is persisted like a top-level run's (a delay, a
    /// capacity wait or a signal deadline arms `scheduled_at`; a capacity wait
    /// also records its provider kind). A child suspended because of its own
    /// child gets no `scheduled_at`: only the deepest run owns the
    /// wake-up. The children of a wave each own theirs: a resume that finds
    /// the root already running waits for it instead of resuming it twice.
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
            EngineError::CapacitySleeping { kind, wake_at, .. } => RunUpdate {
                status: Some(RunStatus::Sleeping),
                scheduled_at: Some(*wake_at),
                capacity_wait_kind: Some(ProviderKind::new(kind.as_str())),
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
