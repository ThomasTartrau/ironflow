//! Workflow guard -- execution limits for agent workflows.
//!
//! Prevents runaway workflows: unbounded sub-workflow depth, uncontrolled
//! fan-out, unlimited token consumption, and infinite execution time.
//! Checked before every agent step invocation.
//!
//! # Examples
//!
//! ```
//! use ironflow_engine::guard::{WorkflowGuardConfig, WorkflowGuardState, WorkflowRejection};
//!
//! let config = WorkflowGuardConfig::default();
//! assert_eq!(config.max_depth, 5);
//!
//! let state = WorkflowGuardState::new();
//! assert!(state.check(&config, "agent-a").is_ok());
//! ```

use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Environment variable prefix for guard configuration.
const ENV_PREFIX: &str = "IRONFLOW_GUARD_";

/// Configurable execution limits for a workflow run.
///
/// Each limit has a sensible default. Limits can be set per-handler via
/// [`WorkflowHandler::guard_config`](crate::handler::WorkflowHandler::guard_config)
/// or globally on the [`Engine`](crate::engine::Engine).
///
/// # Examples
///
/// ```
/// use ironflow_engine::guard::WorkflowGuardConfig;
///
/// let config = WorkflowGuardConfig::new()
///     .with_max_depth(3)
///     .with_max_fan_out(10);
///
/// assert_eq!(config.max_depth, 3);
/// assert_eq!(config.max_fan_out, 10);
/// assert_eq!(config.max_workflow_tokens, 100_000);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowGuardConfig {
    /// Maximum depth of nested sub-workflows (default 5).
    pub max_depth: u32,
    /// Maximum total number of workflow invocations (default 20).
    pub max_fan_out: u32,
    /// Cumulative token budget across all agent steps (default 100,000).
    pub max_workflow_tokens: u64,
    /// Global timeout in seconds (default 120).
    pub workflow_timeout_secs: u64,
}

impl Default for WorkflowGuardConfig {
    fn default() -> Self {
        Self {
            max_depth: 5,
            max_fan_out: 20,
            max_workflow_tokens: 100_000,
            workflow_timeout_secs: 120,
        }
    }
}

impl WorkflowGuardConfig {
    /// Create a configuration with default limits.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::guard::WorkflowGuardConfig;
    ///
    /// assert_eq!(WorkflowGuardConfig::new(), WorkflowGuardConfig::default());
    /// ```
    pub fn new() -> Self {
        Self::default()
    }

    /// Load configuration from environment variables.
    ///
    /// Reads `IRONFLOW_GUARD_MAX_DEPTH`, `IRONFLOW_GUARD_MAX_FAN_OUT`,
    /// `IRONFLOW_GUARD_MAX_WORKFLOW_TOKENS`, and `IRONFLOW_GUARD_WORKFLOW_TIMEOUT_SECS`.
    /// Missing or unparseable variables fall back to defaults.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::guard::WorkflowGuardConfig;
    ///
    /// let config = WorkflowGuardConfig::from_env();
    /// // Without env vars set, returns defaults.
    /// assert_eq!(config.max_depth, 5);
    /// ```
    pub fn from_env() -> Self {
        Self {
            max_depth: parse_env("MAX_DEPTH", 5),
            max_fan_out: parse_env("MAX_FAN_OUT", 20),
            max_workflow_tokens: parse_env("MAX_WORKFLOW_TOKENS", 100_000),
            workflow_timeout_secs: parse_env("WORKFLOW_TIMEOUT_SECS", 120),
        }
    }

    /// Set the maximum sub-workflow nesting depth.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::guard::WorkflowGuardConfig;
    ///
    /// let config = WorkflowGuardConfig::new().with_max_depth(3);
    /// assert_eq!(config.max_depth, 3);
    /// ```
    pub fn with_max_depth(mut self, max_depth: u32) -> Self {
        self.max_depth = max_depth;
        self
    }

    /// Set the maximum total workflow invocations.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::guard::WorkflowGuardConfig;
    ///
    /// let config = WorkflowGuardConfig::new().with_max_fan_out(10);
    /// assert_eq!(config.max_fan_out, 10);
    /// ```
    pub fn with_max_fan_out(mut self, max_fan_out: u32) -> Self {
        self.max_fan_out = max_fan_out;
        self
    }

    /// Set the cumulative token budget.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::guard::WorkflowGuardConfig;
    ///
    /// let config = WorkflowGuardConfig::new().with_max_workflow_tokens(50_000);
    /// assert_eq!(config.max_workflow_tokens, 50_000);
    /// ```
    pub fn with_max_workflow_tokens(mut self, max: u64) -> Self {
        self.max_workflow_tokens = max;
        self
    }

    /// Set the global timeout in seconds.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::guard::WorkflowGuardConfig;
    ///
    /// let config = WorkflowGuardConfig::new().with_workflow_timeout_secs(60);
    /// assert_eq!(config.workflow_timeout_secs, 60);
    /// ```
    pub fn with_workflow_timeout_secs(mut self, secs: u64) -> Self {
        self.workflow_timeout_secs = secs;
        self
    }
}

fn parse_env<T: std::str::FromStr>(suffix: &str, default: T) -> T {
    std::env::var(format!("{ENV_PREFIX}{suffix}"))
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Reason a workflow invocation was rejected by the guard.
///
/// Each variant carries the observed value and the configured limit
/// for diagnostics.
///
/// # Examples
///
/// ```
/// use ironflow_engine::guard::WorkflowRejection;
///
/// let r = WorkflowRejection::MaxDepthExceeded { depth: 6, max: 5 };
/// assert!(r.to_string().contains("6"));
/// assert!(r.to_string().contains("5"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkflowRejection {
    /// The sub-workflow nesting depth exceeds the configured maximum.
    MaxDepthExceeded {
        /// Current depth.
        depth: u32,
        /// Configured limit.
        max: u32,
    },
    /// The target workflow is already in the call chain (direct or indirect cycle).
    CycleDetected {
        /// The workflow that would create a cycle.
        target: String,
        /// The current call chain.
        chain: Vec<String>,
    },
    /// The total number of workflow invocations exceeds the configured maximum.
    MaxFanOutExceeded {
        /// Current invocation count.
        invocations: u32,
        /// Configured limit.
        max: u32,
    },
    /// The cumulative token usage exceeds the configured budget.
    TokenBudgetExhausted {
        /// Tokens already consumed.
        used: u64,
        /// Configured limit.
        max: u64,
    },
    /// The workflow has exceeded its global timeout.
    WorkflowTimeout {
        /// Seconds elapsed since the workflow started.
        elapsed_secs: u64,
        /// Configured limit.
        max: u64,
    },
    /// The guard state is unavailable (fail closed).
    GuardUnavailable,
}

/// Business error code for guard rejections.
pub const WORKFLOW_GUARD_REJECTED_CODE: &str = "WORKFLOW_GUARD_REJECTED";

impl fmt::Display for WorkflowRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MaxDepthExceeded { depth, max } => {
                write!(f, "max call depth exceeded: {depth}/{max}")
            }
            Self::CycleDetected { target, chain } => {
                write!(
                    f,
                    "cycle detected: workflow {target:?} already in chain {chain:?}"
                )
            }
            Self::MaxFanOutExceeded { invocations, max } => {
                write!(f, "max fan-out exceeded: {invocations}/{max} invocations")
            }
            Self::TokenBudgetExhausted { used, max } => {
                write!(f, "token budget exhausted: {used}/{max}")
            }
            Self::WorkflowTimeout { elapsed_secs, max } => {
                write!(f, "workflow timeout: {elapsed_secs}s/{max}s")
            }
            Self::GuardUnavailable => {
                write!(f, "workflow guard unavailable -- failing closed")
            }
        }
    }
}

impl std::error::Error for WorkflowRejection {}

/// Mutable runtime state tracked by the guard during a workflow execution.
///
/// Shared between parent and child workflows via `Arc<Mutex<_>>` so that
/// limits are enforced globally across the entire run tree.
///
/// # Examples
///
/// ```
/// use ironflow_engine::guard::{WorkflowGuardConfig, WorkflowGuardState};
///
/// let mut state = WorkflowGuardState::new();
/// let config = WorkflowGuardConfig::new().with_max_depth(2);
///
/// state.record_invocation("workflow-a");
/// state.record_invocation("workflow-b");
/// assert!(state.check(&config, "workflow-c").is_err());
/// ```
#[derive(Debug)]
pub struct WorkflowGuardState {
    /// Current sub-workflow nesting depth.
    depth: u32,
    /// Call chain for cycle detection (stack of workflow names).
    call_chain: Vec<String>,
    /// Total number of workflow invocations in this run tree.
    total_invocations: u32,
    /// Cumulative tokens consumed across all agent steps.
    total_tokens_used: u64,
    /// When the root workflow started.
    started_at: Instant,
}

impl WorkflowGuardState {
    /// Create a fresh guard state for a new workflow execution.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::guard::WorkflowGuardState;
    ///
    /// let state = WorkflowGuardState::new();
    /// assert_eq!(state.depth(), 0);
    /// assert_eq!(state.total_invocations(), 0);
    /// ```
    pub fn new() -> Self {
        Self {
            depth: 0,
            call_chain: Vec::new(),
            total_invocations: 0,
            total_tokens_used: 0,
            started_at: Instant::now(),
        }
    }

    /// Current sub-workflow nesting depth.
    pub fn depth(&self) -> u32 {
        self.depth
    }

    /// Total workflow invocations so far.
    pub fn total_invocations(&self) -> u32 {
        self.total_invocations
    }

    /// Total tokens consumed so far.
    pub fn total_tokens_used(&self) -> u64 {
        self.total_tokens_used
    }

    /// The current call chain (workflow name stack).
    pub fn call_chain(&self) -> &[String] {
        &self.call_chain
    }

    /// Seconds elapsed since the workflow started.
    pub fn elapsed_secs(&self) -> u64 {
        self.started_at.elapsed().as_secs()
    }

    /// Read-only check: would invoking `target_workflow` violate any limit?
    ///
    /// Does not mutate state. Call this before
    /// [`record_invocation`](Self::record_invocation).
    ///
    /// # Errors
    ///
    /// Returns the specific [`WorkflowRejection`] variant that would be
    /// violated.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::guard::{WorkflowGuardConfig, WorkflowGuardState, WorkflowRejection};
    ///
    /// let state = WorkflowGuardState::new();
    /// let config = WorkflowGuardConfig::new().with_max_depth(0);
    ///
    /// let err = state.check(&config, "child").unwrap_err();
    /// assert!(matches!(err, WorkflowRejection::MaxDepthExceeded { .. }));
    /// ```
    pub fn check(
        &self,
        config: &WorkflowGuardConfig,
        target_workflow: &str,
    ) -> Result<(), WorkflowRejection> {
        if self.depth >= config.max_depth {
            return Err(WorkflowRejection::MaxDepthExceeded {
                depth: self.depth,
                max: config.max_depth,
            });
        }

        if self.call_chain.iter().any(|id| id == target_workflow) {
            return Err(WorkflowRejection::CycleDetected {
                target: target_workflow.to_string(),
                chain: self.call_chain.clone(),
            });
        }

        if self.total_invocations >= config.max_fan_out {
            return Err(WorkflowRejection::MaxFanOutExceeded {
                invocations: self.total_invocations,
                max: config.max_fan_out,
            });
        }

        if self.total_tokens_used >= config.max_workflow_tokens {
            return Err(WorkflowRejection::TokenBudgetExhausted {
                used: self.total_tokens_used,
                max: config.max_workflow_tokens,
            });
        }

        let elapsed = self.started_at.elapsed().as_secs();
        if elapsed >= config.workflow_timeout_secs {
            return Err(WorkflowRejection::WorkflowTimeout {
                elapsed_secs: elapsed,
                max: config.workflow_timeout_secs,
            });
        }

        Ok(())
    }

    /// Record that a sub-workflow invocation is starting.
    ///
    /// Increments depth and fan-out counter, and pushes the workflow name
    /// onto the call chain for cycle detection.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::guard::WorkflowGuardState;
    ///
    /// let mut state = WorkflowGuardState::new();
    /// state.record_invocation("child-workflow");
    /// assert_eq!(state.depth(), 1);
    /// assert_eq!(state.total_invocations(), 1);
    /// assert_eq!(state.call_chain(), &["child-workflow"]);
    /// ```
    pub fn record_invocation(&mut self, target_workflow: &str) {
        self.depth += 1;
        self.total_invocations += 1;
        self.call_chain.push(target_workflow.to_string());
    }

    /// Record that a sub-workflow invocation has returned.
    ///
    /// Decrements depth and pops the last entry from the call chain.
    /// Safe to call even on failure paths (the guard must never leak depth).
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::guard::WorkflowGuardState;
    ///
    /// let mut state = WorkflowGuardState::new();
    /// state.record_invocation("child");
    /// assert_eq!(state.depth(), 1);
    ///
    /// state.record_return();
    /// assert_eq!(state.depth(), 0);
    /// assert!(state.call_chain().is_empty());
    /// ```
    pub fn record_return(&mut self) {
        self.depth = self.depth.saturating_sub(1);
        self.call_chain.pop();
    }

    /// Record tokens consumed by an agent step.
    ///
    /// Returns the remaining token budget, or an error if the budget is
    /// now exhausted.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowRejection::TokenBudgetExhausted`] when the
    /// cumulative usage exceeds the configured maximum.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::guard::{WorkflowGuardConfig, WorkflowGuardState, WorkflowRejection};
    ///
    /// let mut state = WorkflowGuardState::new();
    /// let config = WorkflowGuardConfig::new().with_max_workflow_tokens(100);
    ///
    /// assert!(state.record_tokens(&config, 50).is_ok());
    /// assert!(state.record_tokens(&config, 60).is_err());
    /// ```
    pub fn record_tokens(
        &mut self,
        config: &WorkflowGuardConfig,
        tokens_used: u64,
    ) -> Result<u64, WorkflowRejection> {
        self.total_tokens_used += tokens_used;
        if self.total_tokens_used > config.max_workflow_tokens {
            return Err(WorkflowRejection::TokenBudgetExhausted {
                used: self.total_tokens_used,
                max: config.max_workflow_tokens,
            });
        }
        Ok(config.max_workflow_tokens - self.total_tokens_used)
    }
}

impl Default for WorkflowGuardState {
    fn default() -> Self {
        Self::new()
    }
}

/// Thread-safe handle to a shared [`WorkflowGuardState`].
///
/// Passed from parent to child workflows so that limits are enforced
/// globally across the entire run tree.
pub type SharedGuardState = Arc<Mutex<WorkflowGuardState>>;

/// Create a new shared guard state for a workflow run.
///
/// # Examples
///
/// ```
/// use ironflow_engine::guard::new_shared_guard_state;
///
/// let state = new_shared_guard_state();
/// let locked = state.lock().unwrap();
/// assert_eq!(locked.depth(), 0);
/// ```
pub fn new_shared_guard_state() -> SharedGuardState {
    Arc::new(Mutex::new(WorkflowGuardState::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_documented_values() {
        let config = WorkflowGuardConfig::default();
        assert_eq!(config.max_depth, 5);
        assert_eq!(config.max_fan_out, 20);
        assert_eq!(config.max_workflow_tokens, 100_000);
        assert_eq!(config.workflow_timeout_secs, 120);
    }

    #[test]
    fn new_equals_default() {
        assert_eq!(WorkflowGuardConfig::new(), WorkflowGuardConfig::default());
    }

    #[test]
    fn config_from_env_reads_and_falls_back() {
        // SAFETY: test-only, single-threaded access to env vars.
        // Combined into one test to avoid races between parallel tests.
        unsafe {
            std::env::set_var("IRONFLOW_GUARD_MAX_DEPTH", "10");
            std::env::set_var("IRONFLOW_GUARD_MAX_FAN_OUT", "50");
            std::env::set_var("IRONFLOW_GUARD_MAX_WORKFLOW_TOKENS", "200000");
            std::env::set_var("IRONFLOW_GUARD_WORKFLOW_TIMEOUT_SECS", "300");
        }

        let config = WorkflowGuardConfig::from_env();
        assert_eq!(config.max_depth, 10);
        assert_eq!(config.max_fan_out, 50);
        assert_eq!(config.max_workflow_tokens, 200_000);
        assert_eq!(config.workflow_timeout_secs, 300);

        // SAFETY: test-only cleanup.
        unsafe {
            std::env::remove_var("IRONFLOW_GUARD_MAX_DEPTH");
            std::env::remove_var("IRONFLOW_GUARD_MAX_FAN_OUT");
            std::env::remove_var("IRONFLOW_GUARD_MAX_WORKFLOW_TOKENS");
            std::env::remove_var("IRONFLOW_GUARD_WORKFLOW_TIMEOUT_SECS");
        }

        let fallback = WorkflowGuardConfig::from_env();
        assert_eq!(fallback.max_depth, 5);
        assert_eq!(fallback.max_fan_out, 20);
    }

    #[test]
    fn builder_methods_set_values() {
        let config = WorkflowGuardConfig::new()
            .with_max_depth(3)
            .with_max_fan_out(10)
            .with_max_workflow_tokens(50_000)
            .with_workflow_timeout_secs(60);

        assert_eq!(config.max_depth, 3);
        assert_eq!(config.max_fan_out, 10);
        assert_eq!(config.max_workflow_tokens, 50_000);
        assert_eq!(config.workflow_timeout_secs, 60);
    }

    #[test]
    fn check_rejects_max_depth_exceeded() {
        let config = WorkflowGuardConfig::new().with_max_depth(2);
        let mut state = WorkflowGuardState::new();
        state.record_invocation("a");
        state.record_invocation("b");

        let err = state.check(&config, "c").unwrap_err();
        assert!(matches!(
            err,
            WorkflowRejection::MaxDepthExceeded { depth: 2, max: 2 }
        ));
    }

    #[test]
    fn check_allows_within_depth_limit() {
        let config = WorkflowGuardConfig::new().with_max_depth(2);
        let mut state = WorkflowGuardState::new();
        state.record_invocation("a");

        assert!(state.check(&config, "b").is_ok());
    }

    #[test]
    fn check_detects_direct_cycle() {
        let config = WorkflowGuardConfig::default();
        let mut state = WorkflowGuardState::new();
        state.record_invocation("workflow-a");

        let err = state.check(&config, "workflow-a").unwrap_err();
        match err {
            WorkflowRejection::CycleDetected { target, chain } => {
                assert_eq!(target, "workflow-a");
                assert_eq!(chain, vec!["workflow-a"]);
            }
            other => panic!("expected CycleDetected, got {other:?}"),
        }
    }

    #[test]
    fn check_detects_indirect_cycle() {
        let config = WorkflowGuardConfig::default();
        let mut state = WorkflowGuardState::new();
        state.record_invocation("a");
        state.record_invocation("b");
        state.record_invocation("c");

        let err = state.check(&config, "a").unwrap_err();
        match err {
            WorkflowRejection::CycleDetected { target, chain } => {
                assert_eq!(target, "a");
                assert_eq!(chain, vec!["a", "b", "c"]);
            }
            other => panic!("expected CycleDetected, got {other:?}"),
        }
    }

    #[test]
    fn check_allows_no_cycle() {
        let config = WorkflowGuardConfig::default();
        let mut state = WorkflowGuardState::new();
        state.record_invocation("a");
        state.record_invocation("b");

        assert!(state.check(&config, "c").is_ok());
    }

    #[test]
    fn check_rejects_fan_out_exceeded() {
        let config = WorkflowGuardConfig::new().with_max_fan_out(2);
        let mut state = WorkflowGuardState::new();
        state.record_invocation("a");
        state.record_return();
        state.record_invocation("b");
        state.record_return();

        let err = state.check(&config, "c").unwrap_err();
        assert!(matches!(
            err,
            WorkflowRejection::MaxFanOutExceeded {
                invocations: 2,
                max: 2
            }
        ));
    }

    #[test]
    fn check_rejects_token_budget_exhausted() {
        let config = WorkflowGuardConfig::new().with_max_workflow_tokens(100);
        let mut state = WorkflowGuardState::new();
        let _ = state.record_tokens(&config, 100);

        let err = state.check(&config, "a").unwrap_err();
        assert!(matches!(
            err,
            WorkflowRejection::TokenBudgetExhausted {
                used: 100,
                max: 100
            }
        ));
    }

    #[test]
    fn check_rejects_workflow_timeout() {
        // elapsed().as_secs() truncates to whole seconds, so we use a state
        // whose started_at we can set to the past via the test-only constructor.
        let state = WorkflowGuardState {
            started_at: Instant::now() - std::time::Duration::from_secs(200),
            ..WorkflowGuardState::new()
        };
        let config = WorkflowGuardConfig::new().with_workflow_timeout_secs(120);

        let err = state.check(&config, "a").unwrap_err();
        match err {
            WorkflowRejection::WorkflowTimeout { elapsed_secs, max } => {
                assert!(elapsed_secs >= 120);
                assert_eq!(max, 120);
            }
            other => panic!("expected WorkflowTimeout, got {other:?}"),
        }
    }

    #[test]
    fn check_allows_within_timeout() {
        let config = WorkflowGuardConfig::new().with_workflow_timeout_secs(120);
        let state = WorkflowGuardState::new();
        assert!(state.check(&config, "a").is_ok());
    }

    #[test]
    fn record_invocation_updates_depth_fanout_chain() {
        let mut state = WorkflowGuardState::new();

        state.record_invocation("first");
        assert_eq!(state.depth(), 1);
        assert_eq!(state.total_invocations(), 1);
        assert_eq!(state.call_chain(), &["first"]);

        state.record_invocation("second");
        assert_eq!(state.depth(), 2);
        assert_eq!(state.total_invocations(), 2);
        assert_eq!(state.call_chain(), &["first", "second"]);
    }

    #[test]
    fn record_return_decrements_depth_and_pops_chain() {
        let mut state = WorkflowGuardState::new();
        state.record_invocation("a");
        state.record_invocation("b");

        state.record_return();
        assert_eq!(state.depth(), 1);
        assert_eq!(state.call_chain(), &["a"]);

        state.record_return();
        assert_eq!(state.depth(), 0);
        assert!(state.call_chain().is_empty());
    }

    #[test]
    fn record_return_saturates_at_zero() {
        let mut state = WorkflowGuardState::new();
        state.record_return();
        assert_eq!(state.depth(), 0);
    }

    #[test]
    fn fan_out_counts_even_after_return() {
        let mut state = WorkflowGuardState::new();
        state.record_invocation("a");
        state.record_return();
        state.record_invocation("b");
        state.record_return();

        assert_eq!(state.total_invocations(), 2);
        assert_eq!(state.depth(), 0);
    }

    #[test]
    fn record_tokens_accumulates_and_rejects_over_budget() {
        let config = WorkflowGuardConfig::new().with_max_workflow_tokens(100);
        let mut state = WorkflowGuardState::new();

        let remaining = state.record_tokens(&config, 40).unwrap();
        assert_eq!(remaining, 60);

        let remaining = state.record_tokens(&config, 40).unwrap();
        assert_eq!(remaining, 20);

        let err = state.record_tokens(&config, 30).unwrap_err();
        assert!(matches!(
            err,
            WorkflowRejection::TokenBudgetExhausted {
                used: 110,
                max: 100
            }
        ));
    }

    #[test]
    fn record_tokens_allows_exact_budget() {
        let config = WorkflowGuardConfig::new().with_max_workflow_tokens(100);
        let mut state = WorkflowGuardState::new();

        let remaining = state.record_tokens(&config, 100).unwrap();
        assert_eq!(remaining, 0);
    }

    #[test]
    fn rejection_display_max_depth() {
        let r = WorkflowRejection::MaxDepthExceeded { depth: 6, max: 5 };
        let msg = r.to_string();
        assert!(msg.contains("max call depth exceeded"));
        assert!(msg.contains("6/5"));
    }

    #[test]
    fn rejection_display_cycle() {
        let r = WorkflowRejection::CycleDetected {
            target: "b".to_string(),
            chain: vec!["a".to_string(), "b".to_string()],
        };
        let msg = r.to_string();
        assert!(msg.contains("cycle detected"));
        assert!(msg.contains("\"b\""));
    }

    #[test]
    fn rejection_display_fan_out() {
        let r = WorkflowRejection::MaxFanOutExceeded {
            invocations: 21,
            max: 20,
        };
        let msg = r.to_string();
        assert!(msg.contains("max fan-out exceeded"));
        assert!(msg.contains("21/20"));
    }

    #[test]
    fn rejection_display_token_budget() {
        let r = WorkflowRejection::TokenBudgetExhausted {
            used: 100_001,
            max: 100_000,
        };
        let msg = r.to_string();
        assert!(msg.contains("token budget exhausted"));
        assert!(msg.contains("100001/100000"));
    }

    #[test]
    fn rejection_display_timeout() {
        let r = WorkflowRejection::WorkflowTimeout {
            elapsed_secs: 130,
            max: 120,
        };
        let msg = r.to_string();
        assert!(msg.contains("workflow timeout"));
        assert!(msg.contains("130s/120s"));
    }

    #[test]
    fn rejection_display_unavailable() {
        let r = WorkflowRejection::GuardUnavailable;
        assert!(r.to_string().contains("failing closed"));
    }

    #[test]
    fn shared_guard_state_is_arc_mutex() {
        let shared = new_shared_guard_state();
        let mut state = shared.lock().unwrap();
        state.record_invocation("test");
        assert_eq!(state.depth(), 1);
    }
}
