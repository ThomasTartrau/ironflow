//! Engine error types.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use thiserror::Error;
use uuid::Uuid;

use ironflow_artifacts::error::ArtifactError;
use ironflow_core::error::OperationError;
use ironflow_store::error::StoreError;
use ironflow_store::models::{ConcurrencyLimitError, RunStatus, WorkerTagError};

use crate::guard::{WORKFLOW_GUARD_REJECTED_CODE, WorkflowRejection};

/// Business error code carried by [`EngineError::RunBudgetExceeded`].
pub const RUN_BUDGET_EXCEEDED_CODE: &str = "RUN_BUDGET_EXCEEDED";

/// Business error code carried by [`EngineError::MonthlyBudgetExceeded`].
pub const MONTHLY_BUDGET_EXCEEDED_CODE: &str = "MONTHLY_BUDGET_EXCEEDED";

/// Business error code carried by [`EngineError::ConcurrencyConflict`].
///
/// # Examples
///
/// ```
/// use ironflow_engine::error::CONCURRENCY_CONFLICT_CODE;
///
/// assert_eq!(CONCURRENCY_CONFLICT_CODE, "CONCURRENCY_CONFLICT");
/// ```
pub const CONCURRENCY_CONFLICT_CODE: &str = "CONCURRENCY_CONFLICT";

/// Business error code for handler-version mismatch on retry.
pub const HANDLER_VERSION_MISMATCH_CODE: &str = "HANDLER_VERSION_MISMATCH";

/// Errors produced by the workflow engine.
#[derive(Debug, Error)]
pub enum EngineError {
    /// An operation (Shell, Http, Agent) failed during step execution.
    #[error("operation failed: {0}")]
    Operation(#[from] OperationError),

    /// The backing store returned an error.
    #[error("store error: {0}")]
    Store(StoreError),

    /// Another non-terminal run already holds the requested concurrency key.
    ///
    /// Converted from [`StoreError::ConcurrencyConflict`], so every `?` on a
    /// store call yields this typed variant instead of [`EngineError::Store`].
    #[error("concurrency key {key:?} is held by active run {run_id}")]
    ConcurrencyConflict {
        /// The contested key.
        key: String,
        /// The run holding it.
        run_id: Uuid,
    },

    /// The requested concurrency limits are invalid: an empty or too long
    /// group, a zero limit, or the same group listed twice.
    ///
    /// Converted from [`StoreError::InvalidConcurrencyLimit`], and returned by
    /// [`Engine::enqueue_handler_with_options`](crate::engine::Engine::enqueue_handler_with_options)
    /// before any other check.
    #[error("invalid concurrency limit: {0}")]
    InvalidConcurrencyLimit(ConcurrencyLimitError),

    /// A requested worker tag is invalid: empty, too long, holding a character
    /// outside ASCII alphanumerics and `- _ . : / =`, or one tag too many.
    ///
    /// Converted from [`StoreError::InvalidWorkerTag`], and returned by
    /// [`Engine::enqueue_handler_with_options`](crate::engine::Engine::enqueue_handler_with_options)
    /// before the run is created.
    #[error("invalid worker tag: {0}")]
    InvalidWorkerTag(WorkerTagError),

    /// The workflow definition is invalid.
    #[error("invalid workflow: {0}")]
    InvalidWorkflow(String),

    /// A step configuration could not be deserialized for execution.
    #[error("step config error: {0}")]
    StepConfig(String),

    /// A decision answer was accessed by name with the wrong type or a missing key.
    #[error("decision error: {0}")]
    Decision(#[from] ironflow_core::error::DecisionError),

    /// A decision step was reached but no [`DecisionProvider`](ironflow_core::decision::DecisionProvider)
    /// is wired into the engine.
    #[error(
        "decision step '{step}' requires a decision provider; \
         wire one with Engine::with_decision_provider(...)"
    )]
    NoDecisionProvider {
        /// The decision step that could not run.
        step: String,
    },

    /// JSON serialization error.
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// The run reached its cumulative cost cap before launching an agent step.
    ///
    /// Raised *before* the step is created, so no work and no spend happen.
    /// The engine transitions the run to
    /// [`Cancelled`](ironflow_store::entities::RunStatus::Cancelled).
    #[error(
        "{RUN_BUDGET_EXCEEDED_CODE}: run {run_id} would exceed its cost cap \
         (spent {spent_usd} USD + next step {step_budget_usd} USD > cap {limit_usd} USD)"
    )]
    RunBudgetExceeded {
        /// The run that hit its cap.
        run_id: uuid::Uuid,
        /// The configured cap, in USD.
        limit_usd: Decimal,
        /// Cost already accumulated by this run and its ancestors, in USD.
        spent_usd: Decimal,
        /// Declared budget of the step that was about to run, in USD.
        step_budget_usd: Decimal,
    },

    /// The global monthly cost quota is exhausted; no new run may be created.
    ///
    /// Runs already in flight are never interrupted by this error.
    #[error(
        "{MONTHLY_BUDGET_EXCEEDED_CODE}: monthly cost quota exhausted \
         ({spent_usd} USD spent of {limit_usd} USD)"
    )]
    MonthlyBudgetExceeded {
        /// The configured monthly quota, in USD.
        limit_usd: Decimal,
        /// Cost already spent during the current calendar month, in USD.
        spent_usd: Decimal,
    },

    /// A step declared an output that produced no file.
    ///
    /// Raised only when the step itself succeeded: a declared output that never
    /// materialised is a broken contract, and failing here beats failing later
    /// in whichever step tried to consume it.
    #[error("step {step:?} declared output {pattern:?} but no file matched")]
    MissingArtifact {
        /// Name of the step that declared the output.
        step: String,
        /// The unmatched pattern.
        pattern: String,
    },

    /// A handle was asked for an artifact the step never declared.
    #[error("step {step:?} declares no artifact output named {name:?}")]
    ArtifactNotDeclared {
        /// Name of the step the handle was asked from.
        step: String,
        /// Name of the artifact that was asked for.
        name: String,
    },

    /// A step asked for an artifact that no earlier step produced.
    #[error("no artifact {name:?} produced by step {step:?} before this point")]
    ArtifactNotFound {
        /// Name of the producing step that was searched for.
        step: String,
        /// Name of the artifact that was searched for.
        name: String,
    },

    /// Artifacts were used but no storage backend is configured.
    #[error("artifact storage is not configured: {0}")]
    ArtifactsUnavailable(String),

    /// The artifact storage backend failed.
    #[error("artifact storage error: {0}")]
    Artifact(#[from] ArtifactError),

    /// The run requires human approval before continuing.
    #[error("approval required for run {run_id}, step {step_id}: {message}")]
    ApprovalRequired {
        /// The run that is awaiting approval.
        run_id: uuid::Uuid,
        /// The approval step that triggered the pause.
        step_id: uuid::Uuid,
        /// The approval message.
        message: String,
    },

    /// An approval gate was rejected instead of granted.
    ///
    /// Raised when a [`StepInterceptor`](crate::executor::StepInterceptor) resolves
    /// the gate with [`ApprovalOutcome::Rejected`](crate::executor::ApprovalOutcome::Rejected).
    /// The engine fails the run; the rejection is deterministic, so the run is
    /// never replayed.
    #[error("approval rejected for run {run_id}, step {step_id}: {reason}")]
    ApprovalRejected {
        /// The run that was stopped.
        run_id: uuid::Uuid,
        /// The approval step that was rejected.
        step_id: uuid::Uuid,
        /// Why the gate was refused.
        reason: String,
    },

    /// The run waits for a typed human input before continuing.
    ///
    /// Raised by [`WorkflowContext::human_input`](crate::context::WorkflowContext::human_input)
    /// when no answer has been given yet. The engine transitions the run to
    /// [`AwaitingApproval`](ironflow_store::entities::RunStatus::AwaitingApproval).
    #[error("human input required for run {run_id}, step {step_id}: {message}")]
    HumanInputRequired {
        /// The run that is awaiting input.
        run_id: uuid::Uuid,
        /// The human input step that triggered the pause.
        step_id: uuid::Uuid,
        /// The message displayed to the person answering.
        message: String,
    },

    /// A human input request was rejected instead of answered.
    ///
    /// Returned to the handler by
    /// [`WorkflowContext::human_input`](crate::context::WorkflowContext::human_input),
    /// which decides what happens next. A propagated rejection fails the run,
    /// and the run is never retried.
    #[error("human input rejected for run {run_id}, step {step_id}: {reason}")]
    HumanInputRejected {
        /// The run the input belongs to.
        run_id: uuid::Uuid,
        /// The human input step that was rejected.
        step_id: uuid::Uuid,
        /// Why the input was refused.
        reason: String,
    },

    /// A delay step suspended the run until the given time.
    ///
    /// The engine transitions the run to
    /// [`Sleeping`](ironflow_store::entities::RunStatus::Sleeping) and sets
    /// `scheduled_at` so the worker re-queues it automatically.
    #[error("delay sleeping for run {run_id}, step {step_id}: wake at {wake_at}")]
    DelaySleeping {
        /// The run that is sleeping.
        run_id: uuid::Uuid,
        /// The delay step.
        step_id: uuid::Uuid,
        /// When the run should be woken up.
        wake_at: chrono::DateTime<chrono::Utc>,
    },

    /// An agent step found every targeted provider account rate limited and
    /// suspended the run until capacity comes back.
    ///
    /// The engine transitions the run to
    /// [`Sleeping`](ironflow_store::entities::RunStatus::Sleeping), sets
    /// `scheduled_at` to `wake_at` and records `kind` in
    /// `capacity_wait_kind`, so adding or re-enabling an account of that kind
    /// wakes it early. On wake, the step runs again from zero at the same
    /// position.
    ///
    /// # Examples
    ///
    /// ```
    /// use chrono::Utc;
    /// use ironflow_engine::error::EngineError;
    /// use uuid::Uuid;
    ///
    /// let err = EngineError::CapacitySleeping {
    ///     run_id: Uuid::nil(),
    ///     step_id: Uuid::nil(),
    ///     kind: "claude".to_string(),
    ///     wake_at: Utc::now(),
    /// };
    /// assert!(err.is_suspension());
    /// ```
    #[error("run {run_id} waiting for {kind} capacity at step {step_id}: wake at {wake_at}")]
    CapacitySleeping {
        /// The run that is sleeping.
        run_id: Uuid,
        /// The agent step that found no capacity.
        step_id: Uuid,
        /// Provider kind the run waits for (e.g. `claude`).
        kind: String,
        /// When the run should be woken up.
        wake_at: DateTime<Utc>,
    },

    /// A signal step suspended the run until a matching signal or its deadline.
    ///
    /// Raised by
    /// [`WorkflowContext::wait_for_signal`](crate::context::WorkflowContext::wait_for_signal)
    /// when no signal was received yet. The engine transitions the run to
    /// [`Sleeping`](ironflow_store::entities::RunStatus::Sleeping) with
    /// `scheduled_at` set to the deadline: a delivery wakes it earlier.
    #[error("run {run_id} waiting for signal {name:?} with key {key:?} until {deadline_at}")]
    SignalWaiting {
        /// The run that is waiting.
        run_id: Uuid,
        /// The signal step the run waits on.
        step_id: Uuid,
        /// Name of the signal step.
        step_name: String,
        /// The awaited signal name.
        name: String,
        /// The awaited occurrence key.
        key: String,
        /// When the wait times out.
        deadline_at: DateTime<Utc>,
    },

    /// A child run started by
    /// [`WorkflowContext::workflow`](crate::context::WorkflowContext::workflow)
    /// suspended (approval, human input, delay or signal), and the parent run
    /// is suspended with it.
    ///
    /// The child keeps its own suspension status and the parent's `Workflow`
    /// step stays open. Resuming the child requeues the root run, which
    /// replays and re-enters the same child run.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::error::EngineError;
    /// use uuid::Uuid;
    ///
    /// let err = EngineError::ChildSuspended {
    ///     run_id: Uuid::nil(),
    ///     cause: Box::new(EngineError::HumanInputRequired {
    ///         run_id: Uuid::nil(),
    ///         step_id: Uuid::nil(),
    ///         message: "Answer the questions".to_string(),
    ///     }),
    /// };
    /// assert!(err.is_suspension());
    /// ```
    #[error("child run {run_id} suspended: {cause}")]
    ChildSuspended {
        /// The direct child run that suspended.
        run_id: Uuid,
        /// Why the child suspended: a leaf suspension
        /// ([`ApprovalRequired`](EngineError::ApprovalRequired),
        /// [`HumanInputRequired`](EngineError::HumanInputRequired),
        /// [`DelaySleeping`](EngineError::DelaySleeping),
        /// [`CapacitySleeping`](EngineError::CapacitySleeping),
        /// [`SignalWaiting`](EngineError::SignalWaiting)) or a nested
        /// `ChildSuspended` for a grand-child.
        cause: Box<EngineError>,
    },

    /// The child run of a `Workflow` step was cancelled (`POST /runs/:id/cancel`
    /// on the child itself), while it ran or while its chain was suspended.
    ///
    /// The step fails with this error and so does the parent, unless the step
    /// tolerates the failure with `allow_failure`: it then completes with a
    /// `Cancelled` [`SubWorkflowOutput`](crate::executor::SubWorkflowOutput).
    /// Not retried: a new attempt would start a child the user just stopped.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::error::EngineError;
    /// use uuid::Uuid;
    ///
    /// let err = EngineError::ChildRunCancelled { run_id: Uuid::nil() };
    /// assert!(err.to_string().contains("cancelled"));
    /// ```
    #[error("child run {run_id} was cancelled")]
    ChildRunCancelled {
        /// The cancelled child run.
        run_id: Uuid,
    },

    /// A signal could not be delivered because it is malformed (empty name or
    /// key).
    #[error("invalid signal: {0}")]
    InvalidSignal(String),

    /// A workflow invocation was rejected by the [workflow guard](crate::guard).
    ///
    /// The run is transitioned to
    /// [`Cancelled`](ironflow_store::entities::RunStatus::Cancelled) when this
    /// error is raised.
    #[error("{WORKFLOW_GUARD_REJECTED_CODE}: {0}")]
    WorkflowGuardRejected(#[from] WorkflowRejection),

    /// The step stored at `position` does not match the step the handler just
    /// called: its name or its `StepKind` differ from what was recorded before
    /// the run was suspended.
    ///
    /// Raised instead of silently serving another step's cached output when the
    /// handler's code changed while the run was suspended (a deploy during a
    /// pending approval, human input, decision escalation or delay): step
    /// positions can shift, and without this check every step after the
    /// divergence point would silently receive another step's output, vote or
    /// approval.
    #[error(
        "replay divergence at position {position}: handler called '{expected}' but the run \
         recorded '{recorded}' (handler changed since the run was suspended?)"
    )]
    ReplayDivergence {
        /// The step position where the recorded step and the step just called
        /// stopped matching.
        position: u32,
        /// Identity (`name (kind)`) of the step the handler just called.
        expected: String,
        /// Identity (`name (kind)`) of the step recorded at `position`.
        recorded: String,
    },

    /// The run's handler changed since the run was created or suspended, and the
    /// handler's current version is not declared compatible with the version the
    /// run was created with.
    ///
    /// Checked before any step is replayed, on the same rule the manual retry
    /// endpoint applies (`HANDLER_VERSION_MISMATCH`). Unlike retry, resume has no
    /// `force` override: replaying an incompatible handler's steps risks serving
    /// one step's cached output to another (see
    /// [`ReplayDivergence`](EngineError::ReplayDivergence)).
    #[error(
        "{HANDLER_VERSION_MISMATCH_CODE}: run {run_id} for handler '{workflow_name}' was created \
         with version {run_version}, but the handler is now at version {current_version}; \
         resume refused (no force override for resume)"
    )]
    HandlerVersionMismatch {
        /// The run that cannot be resumed.
        run_id: uuid::Uuid,
        /// The handler's registered name.
        workflow_name: String,
        /// The handler version the run was created with.
        run_version: String,
        /// The handler's current version.
        current_version: String,
    },
}

impl From<StoreError> for EngineError {
    fn from(err: StoreError) -> Self {
        match err {
            StoreError::ConcurrencyConflict { key, run_id } => {
                EngineError::ConcurrencyConflict { key, run_id }
            }
            StoreError::InvalidConcurrencyLimit(e) => EngineError::InvalidConcurrencyLimit(e),
            StoreError::InvalidWorkerTag(e) => EngineError::InvalidWorkerTag(e),
            other => EngineError::Store(other),
        }
    }
}

impl EngineError {
    /// Whether this error suspends the run instead of failing it.
    ///
    /// True for [`ApprovalRequired`](EngineError::ApprovalRequired),
    /// [`HumanInputRequired`](EngineError::HumanInputRequired),
    /// [`DelaySleeping`](EngineError::DelaySleeping),
    /// [`CapacitySleeping`](EngineError::CapacitySleeping),
    /// [`SignalWaiting`](EngineError::SignalWaiting) and
    /// [`ChildSuspended`](EngineError::ChildSuspended).
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::error::EngineError;
    ///
    /// assert!(!EngineError::StepConfig("bad".to_string()).is_suspension());
    /// ```
    pub fn is_suspension(&self) -> bool {
        matches!(
            self,
            EngineError::ApprovalRequired { .. }
                | EngineError::HumanInputRequired { .. }
                | EngineError::DelaySleeping { .. }
                | EngineError::CapacitySleeping { .. }
                | EngineError::SignalWaiting { .. }
                | EngineError::ChildSuspended { .. }
        )
    }

    /// The leaf suspension behind a chain of
    /// [`ChildSuspended`](EngineError::ChildSuspended) errors.
    ///
    /// Returns `self` for any error that is not `ChildSuspended`.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::error::EngineError;
    /// use uuid::Uuid;
    ///
    /// let err = EngineError::ChildSuspended {
    ///     run_id: Uuid::nil(),
    ///     cause: Box::new(EngineError::ApprovalRequired {
    ///         run_id: Uuid::nil(),
    ///         step_id: Uuid::nil(),
    ///         message: "deploy?".to_string(),
    ///     }),
    /// };
    /// assert!(matches!(err.suspension_leaf(), EngineError::ApprovalRequired { .. }));
    /// ```
    pub fn suspension_leaf(&self) -> &EngineError {
        let mut current = self;
        while let EngineError::ChildSuspended { cause, .. } = current {
            current = cause;
        }
        current
    }

    /// The status a run suspended by this error takes: `Sleeping` when the
    /// leaf suspension is a delay, a capacity wait or a signal, `AwaitingApproval` otherwise (a
    /// gate a human resolves).
    pub(crate) fn suspension_status(&self) -> RunStatus {
        match self.suspension_leaf() {
            EngineError::DelaySleeping { .. }
            | EngineError::CapacitySleeping { .. }
            | EngineError::SignalWaiting { .. } => RunStatus::Sleeping,
            _ => RunStatus::AwaitingApproval,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::retry_policy::is_run_retryable;

    #[test]
    fn invalid_workflow_display() {
        let err = EngineError::InvalidWorkflow("unknown-handler".to_string());
        assert!(err.to_string().contains("invalid workflow"));
        assert!(err.to_string().contains("unknown-handler"));
    }

    #[test]
    fn step_config_display() {
        let err = EngineError::StepConfig("bad shell config".to_string());
        assert!(err.to_string().contains("step config error"));
        assert!(err.to_string().contains("bad shell config"));
    }

    #[test]
    fn human_input_required_display() {
        let err = EngineError::HumanInputRequired {
            run_id: uuid::Uuid::nil(),
            step_id: uuid::Uuid::nil(),
            message: "Answer the questions".to_string(),
        };
        let text = err.to_string();
        assert!(text.contains("human input required"));
        assert!(text.contains("Answer the questions"));
    }

    #[test]
    fn human_input_rejected_display() {
        let err = EngineError::HumanInputRejected {
            run_id: uuid::Uuid::nil(),
            step_id: uuid::Uuid::nil(),
            reason: "not relevant".to_string(),
        };
        let text = err.to_string();
        assert!(text.contains("human input rejected"));
        assert!(text.contains("not relevant"));
    }

    #[test]
    fn store_error_from_conversion() {
        let store_err = StoreError::RunNotFound(uuid::Uuid::nil());
        let engine_err = EngineError::from(store_err);
        assert!(engine_err.to_string().contains("store error"));
    }

    #[test]
    fn store_concurrency_conflict_converts_to_engine_variant() {
        let run_id = Uuid::now_v7();
        let engine_err = EngineError::from(StoreError::ConcurrencyConflict {
            key: "issue:12".to_string(),
            run_id,
        });
        match engine_err {
            EngineError::ConcurrencyConflict {
                ref key,
                run_id: holder,
            } => {
                assert_eq!(key, "issue:12");
                assert_eq!(holder, run_id);
            }
            ref other => panic!("expected ConcurrencyConflict, got {other:?}"),
        }
        assert!(engine_err.to_string().contains("issue:12"));
        assert!(!is_run_retryable(&engine_err));
    }

    #[test]
    fn store_invalid_concurrency_limit_converts_to_engine_variant() {
        let engine_err = EngineError::from(StoreError::InvalidConcurrencyLimit(
            ConcurrencyLimitError::ZeroLimit {
                group: "repo:acme".to_string(),
            },
        ));
        assert!(
            matches!(
                engine_err,
                EngineError::InvalidConcurrencyLimit(ConcurrencyLimitError::ZeroLimit { .. })
            ),
            "{engine_err:?}"
        );
        assert!(engine_err.to_string().contains("repo:acme"));
        assert!(!is_run_retryable(&engine_err));
    }

    #[test]
    fn store_invalid_worker_tag_converts_to_engine_variant() {
        let engine_err =
            EngineError::from(StoreError::InvalidWorkerTag(WorkerTagError::InvalidChar {
                tag: "bad,tag".to_string(),
            }));
        assert!(
            matches!(
                engine_err,
                EngineError::InvalidWorkerTag(WorkerTagError::InvalidChar { .. })
            ),
            "{engine_err:?}"
        );
        assert!(engine_err.to_string().contains("bad,tag"));
        assert!(!is_run_retryable(&engine_err));
    }

    #[test]
    fn run_budget_exceeded_display_carries_code_and_amounts() {
        let err = EngineError::RunBudgetExceeded {
            run_id: uuid::Uuid::nil(),
            limit_usd: Decimal::new(200, 2),
            spent_usd: Decimal::new(180, 2),
            step_budget_usd: Decimal::new(50, 2),
        };

        let msg = err.to_string();
        assert!(msg.contains(RUN_BUDGET_EXCEEDED_CODE));
        assert!(msg.contains("2.00"));
        assert!(msg.contains("1.80"));
        assert!(msg.contains("0.50"));
    }

    #[test]
    fn monthly_budget_exceeded_display_carries_code_and_amounts() {
        let err = EngineError::MonthlyBudgetExceeded {
            limit_usd: Decimal::new(10000, 2),
            spent_usd: Decimal::new(10500, 2),
        };

        let msg = err.to_string();
        assert!(msg.contains(MONTHLY_BUDGET_EXCEEDED_CODE));
        assert!(msg.contains("100.00"));
        assert!(msg.contains("105.00"));
    }

    #[test]
    fn missing_artifact_display_names_the_step_and_pattern() {
        let err = EngineError::MissingArtifact {
            step: "build".to_string(),
            pattern: "target/report.html".to_string(),
        };

        let msg = err.to_string();
        assert!(msg.contains("\"build\""));
        assert!(msg.contains("target/report.html"));
    }

    #[test]
    fn artifact_not_declared_display_names_the_step_and_artifact() {
        let err = EngineError::ArtifactNotDeclared {
            step: "build".to_string(),
            name: "report.htm".to_string(),
        };

        let msg = err.to_string();
        assert!(msg.contains("\"build\""));
        assert!(msg.contains("\"report.htm\""));
    }

    #[test]
    fn artifact_not_found_display_names_the_producer() {
        let err = EngineError::ArtifactNotFound {
            step: "build".to_string(),
            name: "report.html".to_string(),
        };

        let msg = err.to_string();
        assert!(msg.contains("\"build\""));
        assert!(msg.contains("report.html"));
    }

    #[test]
    fn artifacts_unavailable_display() {
        let err = EngineError::ArtifactsUnavailable("no blob store".to_string());
        assert!(err.to_string().contains("not configured"));
    }

    #[test]
    fn artifact_error_from_conversion() {
        let engine_err = EngineError::from(ArtifactError::NotFound("a/b".to_string()));
        assert!(engine_err.to_string().contains("artifact storage error"));
    }

    #[test]
    fn serialization_error_from_conversion() {
        let serde_err = serde_json::from_str::<String>("not json").unwrap_err();
        let engine_err = EngineError::from(serde_err);
        assert!(engine_err.to_string().contains("serialization error"));
    }

    #[test]
    fn workflow_guard_rejected_display_carries_code_and_detail() {
        use crate::guard::WorkflowRejection;

        let rejection = WorkflowRejection::MaxDepthExceeded { depth: 6, max: 5 };
        let err = EngineError::from(rejection);

        let msg = err.to_string();
        assert!(msg.contains(WORKFLOW_GUARD_REJECTED_CODE));
        assert!(msg.contains("max call depth exceeded"));
        assert!(msg.contains("6/5"));
    }

    #[test]
    fn workflow_guard_rejected_from_conversion() {
        use crate::guard::WorkflowRejection;

        let rejection = WorkflowRejection::CycleDetected {
            target: "wf-b".to_string(),
            chain: vec!["wf-a".to_string(), "wf-b".to_string()],
        };
        let engine_err = EngineError::from(rejection);
        assert!(engine_err.to_string().contains("cycle detected"));
    }

    #[test]
    fn replay_divergence_display_carries_position_and_identities() {
        let err = EngineError::ReplayDivergence {
            position: 5,
            expected: "resolve-base-branch (Shell)".to_string(),
            recorded: "create-worktree (Shell)".to_string(),
        };

        let msg = err.to_string();
        assert!(msg.contains("divergence"));
        assert!(msg.contains("position 5"));
        assert!(msg.contains("resolve-base-branch"));
        assert!(msg.contains("create-worktree"));
    }

    #[test]
    fn handler_version_mismatch_display_carries_code_and_versions() {
        let err = EngineError::HandlerVersionMismatch {
            run_id: uuid::Uuid::nil(),
            workflow_name: "deploy".to_string(),
            run_version: "1.0.0".to_string(),
            current_version: "2.0.0".to_string(),
        };

        let msg = err.to_string();
        assert!(msg.contains(HANDLER_VERSION_MISMATCH_CODE));
        assert!(msg.contains("1.0.0"));
        assert!(msg.contains("2.0.0"));
    }

    fn human_input_required() -> EngineError {
        EngineError::HumanInputRequired {
            run_id: Uuid::nil(),
            step_id: Uuid::nil(),
            message: "Answer the questions".to_string(),
        }
    }

    #[test]
    fn child_suspended_display_carries_child_and_cause() {
        let child = Uuid::now_v7();
        let err = EngineError::ChildSuspended {
            run_id: child,
            cause: Box::new(human_input_required()),
        };

        let msg = err.to_string();
        assert!(msg.contains(&child.to_string()));
        assert!(msg.contains("human input required"));
    }

    #[test]
    fn leaf_suspensions_and_child_suspended_are_suspensions() {
        let wake_at = Utc::now();
        let suspensions = [
            EngineError::ApprovalRequired {
                run_id: Uuid::nil(),
                step_id: Uuid::nil(),
                message: "deploy?".to_string(),
            },
            human_input_required(),
            EngineError::DelaySleeping {
                run_id: Uuid::nil(),
                step_id: Uuid::nil(),
                wake_at,
            },
            EngineError::CapacitySleeping {
                run_id: Uuid::nil(),
                step_id: Uuid::nil(),
                kind: "claude".to_string(),
                wake_at,
            },
            EngineError::SignalWaiting {
                run_id: Uuid::nil(),
                step_id: Uuid::nil(),
                step_name: "wait".to_string(),
                name: "payment".to_string(),
                key: "order-1".to_string(),
                deadline_at: wake_at,
            },
            EngineError::ChildSuspended {
                run_id: Uuid::nil(),
                cause: Box::new(human_input_required()),
            },
        ];
        for err in &suspensions {
            assert!(err.is_suspension(), "{err} should be a suspension");
        }
    }

    #[test]
    fn failures_and_rejections_are_not_suspensions() {
        let failures = [
            EngineError::InvalidWorkflow("x".to_string()),
            EngineError::HumanInputRejected {
                run_id: Uuid::nil(),
                step_id: Uuid::nil(),
                reason: "no".to_string(),
            },
            EngineError::ApprovalRejected {
                run_id: Uuid::nil(),
                step_id: Uuid::nil(),
                reason: "no".to_string(),
            },
        ];
        for err in &failures {
            assert!(!err.is_suspension(), "{err} should not be a suspension");
        }
    }

    #[test]
    fn suspension_leaf_unwraps_nested_child_suspensions() {
        let err = EngineError::ChildSuspended {
            run_id: Uuid::now_v7(),
            cause: Box::new(EngineError::ChildSuspended {
                run_id: Uuid::now_v7(),
                cause: Box::new(human_input_required()),
            }),
        };

        assert!(matches!(
            err.suspension_leaf(),
            EngineError::HumanInputRequired { .. }
        ));
    }

    #[test]
    fn suspension_status_follows_the_leaf() {
        let human = EngineError::ChildSuspended {
            run_id: Uuid::nil(),
            cause: Box::new(human_input_required()),
        };
        assert_eq!(human.suspension_status(), RunStatus::AwaitingApproval);

        let delay = EngineError::ChildSuspended {
            run_id: Uuid::nil(),
            cause: Box::new(EngineError::DelaySleeping {
                run_id: Uuid::nil(),
                step_id: Uuid::nil(),
                wake_at: Utc::now(),
            }),
        };
        assert_eq!(delay.suspension_status(), RunStatus::Sleeping);

        let capacity = EngineError::ChildSuspended {
            run_id: Uuid::nil(),
            cause: Box::new(EngineError::CapacitySleeping {
                run_id: Uuid::nil(),
                step_id: Uuid::nil(),
                kind: "claude".to_string(),
                wake_at: Utc::now(),
            }),
        };
        assert_eq!(capacity.suspension_status(), RunStatus::Sleeping);
    }

    #[test]
    fn suspension_leaf_of_a_plain_error_is_itself() {
        let err = EngineError::StepConfig("bad".to_string());
        assert!(matches!(err.suspension_leaf(), EngineError::StepConfig(_)));
    }
}
