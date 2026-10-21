//! Preparation of the child run of a `Workflow` step.
//!
//! Starting a child is split in two. [`prepare_child`](WorkflowContext::prepare_child)
//! creates or re-enters the child run and builds its context; it runs in
//! order, so the children of a wave are created in list order. The execution
//! that follows lives in [`child_run`](super::child_run); the children of a
//! wave run it concurrently.

use std::collections::HashMap;
use std::sync::Arc;

use rust_decimal::Decimal;
use serde_json::{Value, json};
use tracing::info;
use uuid::Uuid;

use ironflow_core::provider::LABEL_ROOT_RUN_ID;
use ironflow_store::error::StoreError;
use ironflow_store::models::{
    NewRun, Run, RunStatus, StepUpdate, TriggerKind, normalize_worker_tags,
};

use crate::config::WorkflowStepConfig;
use crate::context::cost::SharedCost;
use crate::context::{PARENT_RUN_ID_LABEL, WorkflowContext, interrupt_running_steps};
use crate::error::EngineError;
use crate::executor::{ConcurrencyConflict, SubWorkflowOutput};
use crate::guard::SharedGuardState;
use crate::handler::{WorkflowHandler, clamp_priority};

use super::child_run::ChildRun;
use super::{CHILD_RUN_ID_KEY, ChildOutcome, ChildResume};

/// A child run ready to execute.
pub(super) struct PreparedChild {
    pub(super) run_id: Uuid,
    pub(super) handler: Arc<dyn WorkflowHandler>,
    pub(super) ctx: WorkflowContext,
    /// The run existed before this step: its completed steps are replayed.
    pub(super) resumed: bool,
}

/// How the child of a `Workflow` step starts.
pub(super) enum ChildStart {
    /// Nothing to execute: a conflict, or a child found already finished.
    Settled(ChildOutcome),
    /// The child run executes.
    Run(Box<PreparedChild>),
}

/// The outcome of a child run found `Cancelled`: with `allow_failure`, a
/// `Cancelled` output carrying the child's error; otherwise
/// [`EngineError::ChildRunCancelled`], which fails the step and the parent.
pub(super) fn cancelled_child_outcome(
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

impl WorkflowContext {
    /// The handler registered as `workflow_name`.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::InvalidWorkflow`] without a handler resolver or
    /// when no handler is registered under that name.
    pub(super) fn resolve_child_handler(
        &self,
        workflow_name: &str,
    ) -> Result<Arc<dyn WorkflowHandler>, EngineError> {
        let resolver = self.handler_resolver.as_ref().ok_or_else(|| {
            EngineError::InvalidWorkflow(
                "sub-workflow requires a handler resolver (use Engine to execute)".to_string(),
            )
        })?;
        resolver(workflow_name).ok_or_else(|| {
            EngineError::InvalidWorkflow(format!("no handler registered: {workflow_name}"))
        })
    }

    /// Execute a child workflow and return aggregated output plus whether
    /// at least one `allow_failure` step failed.
    ///
    /// The child shares this run's guard state and live cost total: nothing
    /// else runs while it does. See [`prepare_child`](Self::prepare_child)
    /// and [`run_child`](Self::run_child).
    pub(super) async fn execute_child_workflow(
        &self,
        config: &WorkflowStepConfig,
        step_id: Uuid,
        resume: Option<ChildResume>,
    ) -> ChildRun {
        let start = match self.resolve_child_handler(&config.workflow_name) {
            Ok(handler) => {
                self.prepare_child(
                    config,
                    step_id,
                    resume,
                    handler,
                    self.guard_state.clone(),
                    self.shared_cost.clone(),
                )
                .await
            }
            Err(err) => Err(err),
        };
        match start {
            Ok(ChildStart::Run(child)) => self.run_child(config, *child).await,
            Ok(ChildStart::Settled(outcome)) => ChildRun::from(Ok(outcome)),
            Err(err) => ChildRun::from(Err(err)),
        }
    }

    /// Create or re-enter the child run of the step `step_id`, and build the
    /// context it executes in.
    ///
    /// When another active run holds the step's concurrency key, no child run
    /// is created and [`ChildOutcome::Conflict`] is returned.
    ///
    /// `resume` is the child run recorded on an open or interrupted step: that
    /// run is re-entered, with its completed steps replayed, instead of
    /// creating a new one. A child re-entered after a lost lease has its
    /// `Running` steps marked interrupted first, like a requeued run, and the
    /// new step records the same child run so a second interruption re-enters
    /// it again. A child found finished is reported as is.
    ///
    /// The child checks the guard against `guard_state` and the cost cap
    /// against `shared_cost`, when set; what it already spent is charged to
    /// that total.
    pub(super) async fn prepare_child(
        &self,
        config: &WorkflowStepConfig,
        step_id: Uuid,
        resume: Option<ChildResume>,
        handler: Arc<dyn WorkflowHandler>,
        guard_state: Option<SharedGuardState>,
        shared_cost: Option<SharedCost>,
    ) -> Result<ChildStart, EngineError> {
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
                    status @ (RunStatus::AwaitingApproval
                    | RunStatus::Pending
                    | RunStatus::Sleeping) => {
                        self.reclaim_child_run(child_run_id, status).await?;
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
                        )
                        .map(ChildStart::Settled);
                    }
                    // The child finished but the parent stopped before closing
                    // its step: report the recorded outcome, run nothing.
                    status @ RunStatus::Failed if config.allow_failure => {
                        let error = child_run
                            .error
                            .clone()
                            .unwrap_or_else(|| "child run failed".to_string());
                        return Ok(ChildStart::Settled(ChildOutcome::Finished(
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
                        )));
                    }
                    status @ (RunStatus::Completed | RunStatus::Warning) => {
                        return Ok(ChildStart::Settled(ChildOutcome::Finished(
                            SubWorkflowOutput::new(
                                child_run_id,
                                &config.workflow_name,
                                status,
                                child_run.cost_usd,
                                child_run.duration_ms,
                            )
                            .with_output(child_run.output.clone()),
                            status == RunStatus::Warning,
                        )));
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
                let priority = config
                    .priority
                    .unwrap_or_else(|| clamp_priority(handler.priority()));
                let child_run_id = match self
                    .create_child_run(config, priority, handler.as_ref())
                    .await
                {
                    Ok(id) => id,
                    Err(EngineError::ConcurrencyConflict { key, run_id }) => {
                        return Ok(ChildStart::Settled(ChildOutcome::Conflict(
                            ConcurrencyConflict::new(key, run_id),
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

        // What a re-entered child already spent belongs to the tree it now
        // shares a live total with.
        if let Some(shared) = &shared_cost {
            shared.charge(carried_cost_usd);
        }

        let ctx = WorkflowContext {
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
            shared_cost,
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
            guard_state,
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
            worker_tags: self.worker_tags.clone(),
            sibling_suspensions: Vec::new(),
        };

        Ok(ChildStart::Run(Box::new(PreparedChild {
            run_id: child_run_id,
            handler,
            ctx,
            resumed: resume.is_some(),
        })))
    }

    /// Move a suspended child run back to `Running` before re-entering it.
    ///
    /// When a sibling resolves while the root is running, a worker or the
    /// waker may move the child between the read of `status` and this write:
    /// a child found already where the transition leads is taken as is.
    async fn reclaim_child_run(
        &self,
        child_run_id: Uuid,
        status: RunStatus,
    ) -> Result<(), EngineError> {
        let path: &[RunStatus] = match status {
            RunStatus::Sleeping => &[RunStatus::Pending, RunStatus::Running],
            _ => &[RunStatus::Running],
        };
        for &next in path {
            match self.store.update_run_status(child_run_id, next).await {
                // Woken by the waker first: go on to `Running`.
                Ok(())
                | Err(StoreError::InvalidTransition {
                    from: RunStatus::Pending,
                    to: RunStatus::Pending,
                }) => {}
                // Picked by a worker that resumes the chain: it waits for this
                // root, which re-enters the child itself.
                Err(StoreError::InvalidTransition {
                    from: RunStatus::Running,
                    ..
                }) => return Ok(()),
                Err(err) => return Err(err.into()),
            }
        }
        Ok(())
    }

    /// Create the child run of a sub-workflow step and return its id.
    ///
    /// The child inherits the parent labels and author, and is linked to its
    /// parent and to the root of the chain by two labels, so a suspended child
    /// can be found and resumed like a top-level run.
    async fn create_child_run(
        &self,
        config: &WorkflowStepConfig,
        priority: i16,
        handler: &dyn WorkflowHandler,
    ) -> Result<Uuid, EngineError> {
        // Whoever triggered the parent workflow is accountable for its children.
        let parent = self.store.get_run(self.run_id).await?;
        let (mut labels, parent_author) =
            parent.map(|r| (r.labels, r.created_by)).unwrap_or_default();
        // The step's own labels win over the inherited ones.
        labels.extend(config.labels.clone());
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
                priority,
                // A child runs inside its parent's slot: it never consumes a
                // concurrency group slot of its own.
                concurrency_limits: Vec::new(),
                // The child shares the parent's cap; it does not get its own budget.
                max_cost_usd: self.max_cost_usd,
                // Recorded for display: the child runs on its parent's worker.
                worker_tags: normalize_worker_tags(handler.required_worker_tags()),
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
}

#[cfg(test)]
mod tests {
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::store::{RunStore, Store};

    use super::*;
    use crate::handler::HandlerFuture;
    use crate::testing::MissingAgentProvider;

    /// A context over a fresh store, and a child run moved along `path`.
    async fn child_moved_along(path: &[RunStatus]) -> (WorkflowContext, Arc<InMemoryStore>, Uuid) {
        let store = Arc::new(InMemoryStore::new());
        let store_dyn: Arc<dyn Store> = store.clone();
        let ctx = WorkflowContext::new(
            Uuid::now_v7(),
            "parent".to_string(),
            store_dyn,
            Arc::new(MissingAgentProvider),
        );
        let config = WorkflowStepConfig::new("child", json!({}));
        let child = ctx
            .create_child_run(&config, 0, &Unregistered)
            .await
            .expect("create the child run");
        for &status in path {
            store
                .update_run_status(child, status)
                .await
                .expect("move the child");
        }
        (ctx, store, child)
    }

    /// A handler only used for its name and worker tags.
    struct Unregistered;

    impl WorkflowHandler for Unregistered {
        fn name(&self) -> &str {
            "child"
        }

        fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
            Box::pin(async { Ok(()) })
        }
    }

    async fn status_of(store: &InMemoryStore, run_id: Uuid) -> RunStatus {
        store
            .get_run(run_id)
            .await
            .expect("get run")
            .expect("run exists")
            .status
            .state
    }

    #[tokio::test]
    async fn a_child_picked_by_a_worker_after_it_was_read_is_taken_as_is() {
        // Read `Pending`, then picked by a worker before this write.
        let (ctx, store, child) = child_moved_along(&[RunStatus::Running]).await;
        ctx.reclaim_child_run(child, RunStatus::Pending)
            .await
            .expect("a running child is re-entered");
        assert_eq!(status_of(&store, child).await, RunStatus::Running);
    }

    #[tokio::test]
    async fn a_child_woken_by_the_waker_after_it_was_read_goes_on_to_running() {
        // Read `Sleeping`, then woken to `Pending` before this write.
        let path = [RunStatus::Running, RunStatus::Sleeping, RunStatus::Pending];
        let (ctx, store, child) = child_moved_along(&path).await;
        ctx.reclaim_child_run(child, RunStatus::Sleeping)
            .await
            .expect("a woken child is re-entered");
        assert_eq!(status_of(&store, child).await, RunStatus::Running);
    }

    #[tokio::test]
    async fn a_child_that_finished_after_it_was_read_is_not_reclaimed() {
        let (ctx, _, child) = child_moved_along(&[RunStatus::Running, RunStatus::Completed]).await;
        let err = ctx
            .reclaim_child_run(child, RunStatus::AwaitingApproval)
            .await
            .expect_err("a finished child cannot run again");
        assert!(matches!(
            err,
            EngineError::Store(StoreError::InvalidTransition {
                from: RunStatus::Completed,
                ..
            })
        ));
    }
}
