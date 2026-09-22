//! Constructors and plain accessors for [`WorkflowContext`].
//!
//! Everything here reads or writes a single field: the run identity, the
//! wiring the engine attaches before execution, the run-level budget, and the
//! counters a handler can inspect while it runs.

use std::collections::HashMap;
use std::sync::Arc;

use rust_decimal::Decimal;
use serde_json::{Value, from_value};
use tracing::error;
use uuid::Uuid;

use ironflow_core::decision::DecisionProvider;
use ironflow_core::provider::AgentProvider;
use ironflow_core::trace_context::WorkflowTraceContext;
use ironflow_store::error::StoreError;
use ironflow_store::models::Step;
use ironflow_store::store::Store;
#[cfg(feature = "secret-store")]
use ironflow_store::workflow_secrets::ScopedSecretStore;

use crate::artifact::ArtifactSink;
use crate::error::EngineError;
use crate::executor::{StepInterceptor, StepResult};
use crate::guard::{SharedGuardState, WorkflowGuardConfig};
use crate::log_sender::LogSender;
use crate::notify::WorkflowEventBus;
#[cfg(not(feature = "secret-store"))]
use crate::operation::NoopSecretResolver;
use crate::operation::{OperationContext, SecretResolver};

use super::{HandlerResolver, WorkflowContext};

impl WorkflowContext {
    /// Create a new context for a run.
    ///
    /// Not typically called directly — the [`Engine`](crate::engine::Engine)
    /// creates this when executing a [`WorkflowHandler`](crate::handler::WorkflowHandler).
    pub fn new(
        run_id: Uuid,
        workflow_name: String,
        store: Arc<dyn Store>,
        provider: Arc<dyn AgentProvider>,
    ) -> Self {
        let trace_context = WorkflowTraceContext::from_workflow_run_id(&run_id.to_string());
        Self {
            run_id,
            workflow_name,
            store,
            provider,
            decision_provider: None,
            handler_resolver: None,
            position: 0,
            last_step_ids: Vec::new(),
            total_cost_usd: Decimal::ZERO,
            total_duration_ms: 0,
            max_cost_usd: None,
            inherited_cost_usd: Decimal::ZERO,
            replay_steps: HashMap::new(),
            granted_approvals: HashMap::new(),
            attempt: 1,
            carried_duration_ms: 0,
            log_sender: None,
            artifact_sink: None,
            has_allowed_failure: false,
            error_handlers: Vec::new(),
            guard_state: None,
            guard_config: None,
            step_results: Vec::new(),
            event_bus: None,
            interceptor: None,
            trace_context,
            operation_ctx: None,
        }
    }

    /// Create a new context with a handler resolver for sub-workflow support.
    ///
    /// The resolver is called when [`workflow`](Self::workflow) is invoked to
    /// look up registered handlers by name.
    pub(crate) fn with_handler_resolver(
        run_id: Uuid,
        workflow_name: String,
        store: Arc<dyn Store>,
        provider: Arc<dyn AgentProvider>,
        resolver: HandlerResolver,
    ) -> Self {
        let trace_context = WorkflowTraceContext::from_workflow_run_id(&run_id.to_string());
        Self {
            run_id,
            workflow_name,
            store,
            provider,
            decision_provider: None,
            handler_resolver: Some(resolver),
            position: 0,
            last_step_ids: Vec::new(),
            total_cost_usd: Decimal::ZERO,
            total_duration_ms: 0,
            max_cost_usd: None,
            inherited_cost_usd: Decimal::ZERO,
            replay_steps: HashMap::new(),
            granted_approvals: HashMap::new(),
            attempt: 1,
            carried_duration_ms: 0,
            log_sender: None,
            artifact_sink: None,
            has_allowed_failure: false,
            error_handlers: Vec::new(),
            guard_state: None,
            guard_config: None,
            step_results: Vec::new(),
            event_bus: None,
            interceptor: None,
            trace_context,
            operation_ctx: None,
        }
    }

    /// Attach a log sender for real-time step output streaming.
    pub fn set_log_sender(&mut self, sender: LogSender) {
        self.log_sender = Some(sender);
    }

    /// Attach the backend that stores and serves artifact bytes.
    ///
    /// Without one, any step that declares an output or calls
    /// [`put_artifact`](Self::put_artifact) fails with
    /// [`EngineError::ArtifactsUnavailable`]. Every other step is unaffected,
    /// so an existing deployment keeps working until artifacts are configured.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::sync::Arc;
    ///
    /// use ironflow_engine::artifact::ArtifactSink;
    /// use ironflow_engine::context::WorkflowContext;
    ///
    /// # fn example(ctx: &mut WorkflowContext, sink: Arc<dyn ArtifactSink>) {
    /// ctx.set_artifact_sink(sink);
    /// # }
    /// ```
    pub fn set_artifact_sink(&mut self, sink: Arc<dyn ArtifactSink>) {
        self.artifact_sink = Some(sink);
    }

    /// Return the W3C trace context for this workflow run.
    ///
    /// The trace context is derived from the run ID and can be used to
    /// correlate spans across distributed services. Each step automatically
    /// receives a [`child`](WorkflowTraceContext::child) context.
    pub fn trace_context(&self) -> &WorkflowTraceContext {
        &self.trace_context
    }

    /// Attach a workflow guard configuration and shared state.
    ///
    /// When set, the guard is checked before every sub-workflow invocation.
    /// The shared state is propagated to child workflows so that limits
    /// apply globally across the entire run tree.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::context::WorkflowContext;
    /// use ironflow_engine::guard::{WorkflowGuardConfig, new_shared_guard_state};
    ///
    /// # fn example(ctx: &mut WorkflowContext) {
    /// ctx.set_guard(WorkflowGuardConfig::default(), new_shared_guard_state());
    /// # }
    /// ```
    pub fn set_guard(&mut self, config: WorkflowGuardConfig, state: SharedGuardState) {
        self.guard_config = Some(config);
        self.guard_state = Some(state);
    }

    /// The current guard configuration, if any.
    pub fn guard_config(&self) -> Option<&WorkflowGuardConfig> {
        self.guard_config.as_ref()
    }

    /// Attach a [`WorkflowEventBus`] for per-run real-time monitoring.
    ///
    /// When set, step transitions automatically publish
    /// [`WorkflowEvent`](crate::notify::WorkflowEvent)s to the bus.
    pub fn set_event_bus(&mut self, bus: WorkflowEventBus) {
        self.event_bus = Some(bus);
    }

    /// Attach a [`StepInterceptor`] that resolves steps without executing them.
    ///
    /// Wired by the [`Engine`](crate::engine::Engine) from
    /// [`Engine::with_step_interceptor`](crate::engine::Engine::with_step_interceptor).
    /// Intended for tests: see [`crate::testing::TestEngine`].
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::sync::Arc;
    ///
    /// use ironflow_engine::context::WorkflowContext;
    /// use ironflow_engine::executor::StepInterceptor;
    ///
    /// # fn example(ctx: &mut WorkflowContext, interceptor: Arc<dyn StepInterceptor>) {
    /// ctx.set_step_interceptor(interceptor);
    /// # }
    /// ```
    pub fn set_step_interceptor(&mut self, interceptor: Arc<dyn StepInterceptor>) {
        self.interceptor = Some(interceptor);
    }

    /// The step interceptor attached to this context, if any.
    pub(crate) fn step_interceptor(&self) -> Option<&Arc<dyn StepInterceptor>> {
        self.interceptor.as_ref()
    }

    /// Attach a [`DecisionProvider`] backend for `ctx.decision(...)` steps.
    ///
    /// Not typically called directly -- the [`Engine`](crate::engine::Engine)
    /// wires this from [`Engine::with_decision_provider`](crate::engine::Engine::with_decision_provider).
    pub fn set_decision_provider(&mut self, provider: Arc<dyn DecisionProvider>) {
        self.decision_provider = Some(provider);
    }

    /// Seed the context with the run's attempt number and the totals already
    /// accumulated by previous attempts.
    ///
    /// Called by the engine before executing a handler. Steps created by this
    /// context belong to `attempt`, and the cost and duration it reports at the
    /// end cover the whole run, not just this attempt.
    pub(crate) fn carry_over_run_totals(
        &mut self,
        attempt: u32,
        cost_usd: Decimal,
        duration_ms: u64,
    ) {
        self.attempt = attempt;
        self.total_cost_usd = cost_usd;
        self.carried_duration_ms = duration_ms;
    }

    /// Wall-clock duration already recorded on the run by previous attempts.
    pub(crate) fn carried_duration_ms(&self) -> u64 {
        self.carried_duration_ms
    }

    /// The run attempt this context is executing (1-based).
    pub fn attempt(&self) -> u32 {
        self.attempt
    }

    /// Set the cumulative cost cap enforced before every agent step.
    ///
    /// Called by the [`Engine`](crate::engine::Engine) with the run's persisted
    /// `max_cost_usd`. `None` disables the check.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::context::WorkflowContext;
    /// use rust_decimal::Decimal;
    ///
    /// # fn example(ctx: &mut WorkflowContext) {
    /// ctx.set_max_cost_usd(Some(Decimal::new(200, 2))); // $2.00
    /// # }
    /// ```
    pub fn set_max_cost_usd(&mut self, cap: Option<Decimal>) {
        self.max_cost_usd = cap;
    }

    /// The cumulative cost cap of this run, if any.
    pub fn max_cost_usd(&self) -> Option<Decimal> {
        self.max_cost_usd
    }

    /// Total cost charged against the cap: this run plus every ancestor run.
    ///
    /// For a top-level run this equals [`total_cost_usd`](Self::total_cost_usd).
    /// For a sub-workflow it also includes what the parent chain already spent.
    pub fn charged_cost_usd(&self) -> Decimal {
        self.inherited_cost_usd + self.total_cost_usd
    }

    /// Reject the upcoming agent work when it would cross the run's cost cap.
    ///
    /// `step_budget` is the declared budget of the step (or the sum of budgets
    /// for a parallel wave). Called *before* any step record is created so a
    /// refused run never launches the work it could not afford.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::RunBudgetExceeded`] when
    /// `charged_cost + step_budget` exceeds the cap.
    pub(super) fn check_run_budget(&self, step_budget: Decimal) -> Result<(), EngineError> {
        let Some(limit) = self.max_cost_usd else {
            return Ok(());
        };

        let spent = self.charged_cost_usd();
        if spent + step_budget <= limit {
            return Ok(());
        }

        error!(
            run_id = %self.run_id,
            limit_usd = %limit,
            spent_usd = %spent,
            step_budget_usd = %step_budget,
            "run cost cap reached, refusing agent step"
        );

        Err(EngineError::RunBudgetExceeded {
            run_id: self.run_id,
            limit_usd: limit,
            spent_usd: spent,
            step_budget_usd: step_budget,
        })
    }

    /// The run ID this context is executing for.
    pub fn run_id(&self) -> Uuid {
        self.run_id
    }

    /// The workflow name this run belongs to.
    pub fn workflow_name(&self) -> &str {
        &self.workflow_name
    }

    /// Accumulated cost across all executed steps so far.
    pub fn total_cost_usd(&self) -> Decimal {
        self.total_cost_usd
    }

    /// Whether at least one `allow_failure` step failed during this run.
    pub fn has_allowed_failure(&self) -> bool {
        self.has_allowed_failure
    }

    /// Accumulated duration across all executed steps so far.
    pub fn total_duration_ms(&self) -> u64 {
        self.total_duration_ms
    }

    /// Enriched results of all completed steps in execution order.
    pub fn step_results(&self) -> &[StepResult] {
        &self.step_results
    }

    /// Return a [`ScopedSecretStore`] scoped to this workflow.
    ///
    /// Secrets are namespaced under `workflows/<uuid>/` where the UUID is
    /// deterministically derived from the workflow name (UUID v5). This means
    /// all runs of the same workflow share the same secret namespace.
    ///
    /// Requires the `secret-store` feature.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::context::WorkflowContext;
    /// use ironflow_engine::error::EngineError;
    ///
    /// # async fn example(ctx: &WorkflowContext) -> Result<(), EngineError> {
    /// let secrets = ctx.secrets();
    /// secrets.set("api_token", "sk-ant-12345").await.map_err(EngineError::Store)?;
    /// let token = secrets.get("api_token").await.map_err(EngineError::Store)?;
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(feature = "secret-store")]
    pub fn secrets(&self) -> ScopedSecretStore {
        let workflow_uuid = Uuid::new_v5(&Uuid::NAMESPACE_OID, self.workflow_name.as_bytes());
        ScopedSecretStore::for_workflow(workflow_uuid, self.store.clone())
    }

    pub(super) fn ensure_operation_ctx(&mut self) -> &OperationContext {
        self.operation_ctx.get_or_insert_with(|| {
            #[cfg(feature = "secret-store")]
            let secrets: Arc<dyn SecretResolver> = {
                let workflow_uuid =
                    Uuid::new_v5(&Uuid::NAMESPACE_OID, self.workflow_name.as_bytes());
                Arc::new(ScopedSecretStore::for_workflow(
                    workflow_uuid,
                    self.store.clone(),
                ))
            };
            #[cfg(not(feature = "secret-store"))]
            let secrets: Arc<dyn SecretResolver> = Arc::new(NoopSecretResolver);

            OperationContext::new(secrets)
        })
    }

    /// Access the store directly (advanced usage).
    pub fn store(&self) -> &Arc<dyn Store> {
        &self.store
    }

    /// Get and increment the current position counter.
    pub(crate) fn next_position(&mut self) -> u32 {
        let pos = self.position;
        self.position += 1;
        pos
    }

    /// Access the replay steps from a previous execution.
    pub(crate) fn replay_steps(&self) -> &HashMap<u32, Step> {
        &self.replay_steps
    }

    /// Set the last step IDs (for dependency tracking).
    pub(crate) fn set_last_step_ids(&mut self, ids: Vec<Uuid>) {
        self.last_step_ids = ids;
    }

    /// Access the payload that triggered this run.
    ///
    /// Fetches the run from the store and returns its payload.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::Store`] if the run is not found.
    pub async fn payload(&self) -> Result<Value, EngineError> {
        let run = self
            .store
            .get_run(self.run_id)
            .await?
            .ok_or(EngineError::Store(StoreError::RunNotFound(self.run_id)))?;
        Ok(run.payload)
    }

    /// Deserialize the run payload into a typed input struct.
    ///
    /// Shorthand for `serde_json::from_value(ctx.payload().await?)`.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::Store`] if the run is not found, or
    /// [`EngineError::Serialization`] if the payload does not match `T`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use ironflow_engine::context::WorkflowContext;
    /// # use ironflow_engine::error::EngineError;
    /// use serde::Deserialize;
    ///
    /// #[derive(Deserialize)]
    /// struct DeployInput {
    ///     environment: String,
    ///     dry_run: Option<bool>,
    /// }
    ///
    /// # async fn example(ctx: &WorkflowContext) -> Result<(), EngineError> {
    /// let input: DeployInput = ctx.input().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn input<T: serde::de::DeserializeOwned>(&self) -> Result<T, EngineError> {
        let payload = self.payload().await?;
        from_value(payload).map_err(EngineError::Serialization)
    }
}
