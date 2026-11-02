//! Workflow guard -- execution limits for agent workflows.
//!
//! Prevents runaway workflows: unbounded sub-workflow depth, uncontrolled
//! fan-out, unlimited token consumption, and infinite execution time.
//! Checked before every agent step invocation.
//!
//! Tokens and active execution time add up over every execution of a run
//! (resume, requeue, retry): the engine seeds the state of each execution
//! from what earlier ones stored. An optional absolute deadline
//! ([`WorkflowGuardConfig::run_deadline`]) also stops a run that sleeps.
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
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, TimeDelta, Utc};

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
    /// Cumulative token budget across all agent steps and every execution
    /// of the run (default 100,000).
    pub max_workflow_tokens: u64,
    /// Global timeout in seconds of active execution time, summed over every
    /// execution of the run, sleeping excluded (default 120).
    pub workflow_timeout_secs: u64,
    /// Absolute deadline of a run, counted from its creation
    /// (`Run.created_at`), sleeping time included (default `None`).
    pub run_deadline: Option<Duration>,
}

impl Default for WorkflowGuardConfig {
    fn default() -> Self {
        Self {
            max_depth: 5,
            max_fan_out: 20,
            max_workflow_tokens: 100_000,
            workflow_timeout_secs: 120,
            run_deadline: None,
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
            run_deadline: None,
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

    /// Set the absolute deadline of a run, counted from its creation.
    ///
    /// Unlike the timeout, the deadline includes the time the run spends
    /// sleeping (waiting for an approval, a signal or a timer): a run still
    /// alive `deadline` after its creation is cancelled.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    ///
    /// use ironflow_engine::guard::WorkflowGuardConfig;
    ///
    /// let week = Duration::from_secs(7 * 24 * 3600);
    /// let config = WorkflowGuardConfig::new().with_run_deadline(week);
    /// assert_eq!(config.run_deadline, Some(week));
    /// ```
    pub fn with_run_deadline(mut self, deadline: Duration) -> Self {
        self.run_deadline = Some(deadline);
        self
    }

    /// The instant a run created at `created_at` reaches its deadline.
    ///
    /// `None` without [`run_deadline`](Self::run_deadline), or when the
    /// deadline cannot be represented as a date.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    ///
    /// use chrono::{TimeDelta, Utc};
    /// use ironflow_engine::guard::WorkflowGuardConfig;
    ///
    /// let created_at = Utc::now();
    /// let config = WorkflowGuardConfig::new().with_run_deadline(Duration::from_secs(60));
    /// assert_eq!(
    ///     config.deadline_for(created_at),
    ///     Some(created_at + TimeDelta::seconds(60))
    /// );
    /// assert_eq!(WorkflowGuardConfig::new().deadline_for(created_at), None);
    /// ```
    pub fn deadline_for(&self, created_at: DateTime<Utc>) -> Option<DateTime<Utc>> {
        self.run_deadline
            .and_then(|d| TimeDelta::from_std(d).ok())
            .and_then(|d| created_at.checked_add_signed(d))
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
    /// The run reached the absolute deadline set by
    /// [`WorkflowGuardConfig::run_deadline`].
    RunDeadlineReached {
        /// When the deadline was reached.
        deadline_at: DateTime<Utc>,
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
            Self::RunDeadlineReached { deadline_at } => {
                write!(f, "run deadline reached at {}", deadline_at.to_rfc3339())
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
    /// Counters of the whole run tree, shared with every branch.
    totals: Arc<GuardTotals>,
    /// When the current execution of the root workflow started.
    started_at: Instant,
    /// Active time spent by earlier executions of the run.
    carried_elapsed: Duration,
    /// Absolute deadline of the run, if any.
    deadline_at: Option<DateTime<Utc>>,
}

/// Counters a run tree shares across the branches of its concurrent
/// sub-workflows.
#[derive(Debug, Default)]
struct GuardTotals {
    /// Total number of workflow invocations in this run tree.
    invocations: AtomicU32,
    /// Cumulative tokens consumed across all agent steps.
    tokens_used: AtomicU64,
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
            totals: Arc::new(GuardTotals::default()),
            started_at: Instant::now(),
            carried_elapsed: Duration::ZERO,
            deadline_at: None,
        }
    }

    /// Seed the tokens already used by earlier executions of the run tree.
    ///
    /// Call it before the state is shared.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::guard::WorkflowGuardState;
    ///
    /// let state = WorkflowGuardState::new().with_tokens_used(700);
    /// assert_eq!(state.total_tokens_used(), 700);
    /// ```
    pub fn with_tokens_used(self, tokens: u64) -> Self {
        self.totals.tokens_used.store(tokens, Ordering::Relaxed);
        self
    }

    /// Seed the active time spent by earlier executions of the run.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    ///
    /// use ironflow_engine::guard::WorkflowGuardState;
    ///
    /// let state = WorkflowGuardState::new().with_elapsed(Duration::from_secs(30));
    /// assert!(state.elapsed_secs() >= 30);
    /// ```
    pub fn with_elapsed(mut self, elapsed: Duration) -> Self {
        self.carried_elapsed = elapsed;
        self
    }

    /// Set the absolute deadline of the run.
    ///
    /// # Examples
    ///
    /// ```
    /// use chrono::Utc;
    /// use ironflow_engine::guard::WorkflowGuardState;
    ///
    /// let at = Utc::now();
    /// let state = WorkflowGuardState::new().with_deadline_at(at);
    /// assert_eq!(state.deadline_at(), Some(at));
    /// ```
    pub fn with_deadline_at(mut self, deadline_at: DateTime<Utc>) -> Self {
        self.deadline_at = Some(deadline_at);
        self
    }

    /// The absolute deadline of the run, if any.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::guard::WorkflowGuardState;
    ///
    /// assert_eq!(WorkflowGuardState::new().deadline_at(), None);
    /// ```
    pub fn deadline_at(&self) -> Option<DateTime<Utc>> {
        self.deadline_at
    }

    /// The state of a `target_workflow` child running concurrently with its
    /// siblings, its invocation already claimed by
    /// [`enter_wave`](Self::enter_wave).
    ///
    /// The branch is one level below this state, `target_workflow` on top of
    /// its call chain, and keeps its own depth and chain from then on, so
    /// siblings never push to, or pop from, each other's chain. Invocations,
    /// tokens and the start time stay shared with the whole run tree.
    pub(crate) fn branch_into(&self, target_workflow: &str) -> Self {
        let mut call_chain = self.call_chain.clone();
        call_chain.push(target_workflow.to_string());
        Self {
            depth: self.depth + 1,
            call_chain,
            totals: self.totals.clone(),
            started_at: self.started_at,
            carried_elapsed: self.carried_elapsed,
            deadline_at: self.deadline_at,
        }
    }

    /// Current sub-workflow nesting depth.
    pub fn depth(&self) -> u32 {
        self.depth
    }

    /// Total workflow invocations so far.
    pub fn total_invocations(&self) -> u32 {
        self.totals.invocations.load(Ordering::Relaxed)
    }

    /// Total tokens consumed so far.
    pub fn total_tokens_used(&self) -> u64 {
        self.totals.tokens_used.load(Ordering::Relaxed)
    }

    /// The current call chain (workflow name stack).
    pub fn call_chain(&self) -> &[String] {
        &self.call_chain
    }

    /// Seconds of active execution time across every execution of the run,
    /// sleeping excluded.
    pub fn elapsed_secs(&self) -> u64 {
        self.elapsed().as_secs()
    }

    fn elapsed(&self) -> Duration {
        let current = self.started_at.elapsed();
        self.carried_elapsed.saturating_add(current)
    }

    /// Check the workflow timeout, then the run deadline.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowRejection::WorkflowTimeout`] once the active time
    /// reaches the timeout, [`WorkflowRejection::RunDeadlineReached`] once
    /// the deadline is past.
    pub(crate) fn check_time(&self, config: &WorkflowGuardConfig) -> Result<(), WorkflowRejection> {
        let elapsed_secs = self.elapsed_secs();
        if elapsed_secs >= config.workflow_timeout_secs {
            return Err(WorkflowRejection::WorkflowTimeout {
                elapsed_secs,
                max: config.workflow_timeout_secs,
            });
        }
        if let Some(deadline_at) = self.deadline_at
            && Utc::now() >= deadline_at
        {
            return Err(WorkflowRejection::RunDeadlineReached { deadline_at });
        }
        Ok(())
    }

    /// The time left before the nearest time limit (timeout or deadline),
    /// and the rejection to raise when it runs out.
    pub(crate) fn time_limit(&self, config: &WorkflowGuardConfig) -> (Duration, WorkflowRejection) {
        let max = Duration::from_secs(config.workflow_timeout_secs);
        let timeout = (
            max.saturating_sub(self.elapsed()),
            WorkflowRejection::WorkflowTimeout {
                elapsed_secs: config.workflow_timeout_secs,
                max: config.workflow_timeout_secs,
            },
        );
        let Some(deadline_at) = self.deadline_at else {
            return timeout;
        };
        // A negative delta means the deadline is already past.
        let left = (deadline_at - Utc::now()).to_std();
        let left = left.unwrap_or(Duration::ZERO);
        if left < timeout.0 {
            (left, WorkflowRejection::RunDeadlineReached { deadline_at })
        } else {
            timeout
        }
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

        let invocations = self.total_invocations();
        if invocations >= config.max_fan_out {
            return Err(WorkflowRejection::MaxFanOutExceeded {
                invocations,
                max: config.max_fan_out,
            });
        }

        let used = self.total_tokens_used();
        if used >= config.max_workflow_tokens {
            return Err(WorkflowRejection::TokenBudgetExhausted {
                used,
                max: config.max_workflow_tokens,
            });
        }

        self.check_time(config)
    }

    /// [`check`](Self::check) `target_workflow`, then claim its invocation
    /// and record it like [`record_invocation`](Self::record_invocation).
    ///
    /// The fan-out slot is claimed atomically: branches running concurrently
    /// never cross `max_fan_out` together.
    ///
    /// # Errors
    ///
    /// Returns the [`WorkflowRejection`] the invocation would violate; nothing
    /// is recorded then.
    pub(crate) fn enter(
        &mut self,
        config: &WorkflowGuardConfig,
        target_workflow: &str,
    ) -> Result<(), WorkflowRejection> {
        self.enter_wave(config, target_workflow, 1)?;
        self.depth += 1;
        self.call_chain.push(target_workflow.to_string());
        Ok(())
    }

    /// [`check`](Self::check) a wave of `count` concurrent invocations of
    /// `target_workflow`, then claim all of them at once. Each child then
    /// runs on its own [`branch_into`](Self::branch_into).
    ///
    /// # Errors
    ///
    /// Returns the [`WorkflowRejection`] the wave would violate; a wave that
    /// does not fit whole is refused whole and claims nothing.
    pub(crate) fn enter_wave(
        &self,
        config: &WorkflowGuardConfig,
        target_workflow: &str,
        count: u32,
    ) -> Result<(), WorkflowRejection> {
        self.check(config, target_workflow)?;
        self.totals
            .invocations
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |taken| {
                taken
                    .checked_add(count)
                    .filter(|total| *total <= config.max_fan_out)
            })
            .map(|_| ())
            .map_err(|taken| WorkflowRejection::MaxFanOutExceeded {
                invocations: taken.saturating_add(count),
                max: config.max_fan_out,
            })
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
        self.totals.invocations.fetch_add(1, Ordering::Relaxed);
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
        let used = self
            .totals
            .tokens_used
            .fetch_add(tokens_used, Ordering::Relaxed)
            .saturating_add(tokens_used);
        if used > config.max_workflow_tokens {
            return Err(WorkflowRejection::TokenBudgetExhausted {
                used,
                max: config.max_workflow_tokens,
            });
        }
        Ok(config.max_workflow_tokens - used)
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
        assert_eq!(config.run_deadline, None);
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
            .with_workflow_timeout_secs(60)
            .with_run_deadline(Duration::from_secs(3600));

        assert_eq!(config.max_depth, 3);
        assert_eq!(config.max_fan_out, 10);
        assert_eq!(config.max_workflow_tokens, 50_000);
        assert_eq!(config.workflow_timeout_secs, 60);
        assert_eq!(config.run_deadline, Some(Duration::from_secs(3600)));
    }

    #[test]
    fn with_tokens_used_counts_toward_the_budget() {
        let config = WorkflowGuardConfig::new().with_max_workflow_tokens(1500);
        let mut state = WorkflowGuardState::new().with_tokens_used(700);

        assert_eq!(state.record_tokens(&config, 800), Ok(0));
        assert!(matches!(
            state.record_tokens(&config, 1),
            Err(WorkflowRejection::TokenBudgetExhausted {
                used: 1501,
                max: 1500
            })
        ));
    }

    #[test]
    fn check_rejects_timeout_reached_by_earlier_executions() {
        let config = WorkflowGuardConfig::new().with_workflow_timeout_secs(120);
        let elapsed = Duration::from_secs(200);
        let state = WorkflowGuardState::new().with_elapsed(elapsed);

        let err = state.check(&config, "a").unwrap_err();
        match err {
            WorkflowRejection::WorkflowTimeout { elapsed_secs, max } => {
                assert!(elapsed_secs >= 200);
                assert_eq!(max, 120);
            }
            other => panic!("expected WorkflowTimeout, got {other:?}"),
        }
    }

    #[test]
    fn check_rejects_a_past_deadline() {
        let deadline_at = Utc::now() - TimeDelta::seconds(1);
        let state = WorkflowGuardState::new().with_deadline_at(deadline_at);

        assert_eq!(
            state.check(&WorkflowGuardConfig::new(), "a"),
            Err(WorkflowRejection::RunDeadlineReached { deadline_at })
        );
    }

    #[test]
    fn check_allows_a_future_deadline() {
        let deadline_at = Utc::now() + TimeDelta::hours(1);
        let state = WorkflowGuardState::new().with_deadline_at(deadline_at);
        assert!(state.check(&WorkflowGuardConfig::new(), "a").is_ok());
    }

    #[test]
    fn time_limit_picks_the_nearest_limit() {
        let config = WorkflowGuardConfig::new().with_workflow_timeout_secs(120);
        let deadline_at = Utc::now() + TimeDelta::seconds(2);
        let state = WorkflowGuardState::new().with_deadline_at(deadline_at);

        let (left, rejection) = state.time_limit(&config);
        assert!(left <= Duration::from_secs(2));
        assert_eq!(
            rejection,
            WorkflowRejection::RunDeadlineReached { deadline_at }
        );

        let (left, rejection) = WorkflowGuardState::new().time_limit(&config);
        assert!(left <= Duration::from_secs(120));
        assert!(left > Duration::from_secs(100));
        assert!(matches!(
            rejection,
            WorkflowRejection::WorkflowTimeout { max: 120, .. }
        ));
    }

    #[test]
    fn time_limit_is_zero_past_the_deadline() {
        let config = WorkflowGuardConfig::new();
        let deadline_at = Utc::now() - TimeDelta::seconds(5);
        let state = WorkflowGuardState::new().with_deadline_at(deadline_at);

        assert_eq!(
            state.time_limit(&config),
            (
                Duration::ZERO,
                WorkflowRejection::RunDeadlineReached { deadline_at }
            )
        );
    }

    #[test]
    fn branch_shares_carried_time_and_deadline() {
        let deadline_at = Utc::now() + TimeDelta::hours(1);
        let root = WorkflowGuardState::new()
            .with_elapsed(Duration::from_secs(50))
            .with_deadline_at(deadline_at);
        let branch = root.branch_into("child");

        assert!(branch.elapsed_secs() >= 50);
        assert_eq!(branch.deadline_at(), Some(deadline_at));
    }

    #[test]
    fn deadline_for_adds_the_deadline_to_creation() {
        let created_at = Utc::now();
        let deadline = Duration::from_secs(90);
        let config = WorkflowGuardConfig::new().with_run_deadline(deadline);
        assert_eq!(
            config.deadline_for(created_at),
            Some(created_at + TimeDelta::seconds(90))
        );
    }

    #[test]
    fn deadline_for_none_without_deadline() {
        assert_eq!(WorkflowGuardConfig::new().deadline_for(Utc::now()), None);
    }

    #[test]
    fn deadline_for_none_on_overflow() {
        let config = WorkflowGuardConfig::new().with_run_deadline(Duration::MAX);
        assert_eq!(config.deadline_for(Utc::now()), None);
    }

    #[test]
    fn rejection_display_run_deadline() {
        let deadline_at = Utc::now();
        let rejection = WorkflowRejection::RunDeadlineReached { deadline_at };
        let msg = rejection.to_string();
        assert!(msg.contains("run deadline reached at"));
        assert!(msg.contains(&deadline_at.to_rfc3339()));
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

    #[test]
    fn a_branch_keeps_its_own_chain_and_shares_the_totals() {
        let mut root = WorkflowGuardState::new();
        root.record_invocation("parent");
        let config = WorkflowGuardConfig::new().with_max_workflow_tokens(10);
        root.enter_wave(&config, "child", 2).expect("room for two");
        let mut left = root.branch_into("child");
        let mut right = root.branch_into("child");
        left.enter(&config, "leaf").expect("no cycle on the left");

        assert_eq!(left.call_chain(), ["parent", "child", "leaf"]);
        assert_eq!(right.call_chain(), ["parent", "child"]);
        assert_eq!(right.depth(), 2);
        assert_eq!(root.call_chain(), ["parent"]);
        assert_eq!(root.total_invocations(), 4);

        assert_eq!(left.record_tokens(&config, 6), Ok(4));
        assert!(
            right.record_tokens(&config, 6).is_err(),
            "tokens are shared"
        );
    }

    #[test]
    fn enter_wave_claims_room_for_every_child_or_none() {
        let mut state = WorkflowGuardState::new();
        state.record_invocation("earlier");
        state.record_return();
        let config = WorkflowGuardConfig::new().with_max_fan_out(3);

        assert_eq!(
            state.enter_wave(&config, "child", 3),
            Err(WorkflowRejection::MaxFanOutExceeded {
                invocations: 4,
                max: 3
            })
        );
        assert_eq!(
            state.total_invocations(),
            1,
            "a refused wave claims nothing"
        );
        assert_eq!(state.enter_wave(&config, "child", 2), Ok(()));
        assert_eq!(state.total_invocations(), 3);
    }

    #[test]
    fn enter_claims_the_last_slot_once_across_branches() {
        let root = WorkflowGuardState::new();
        let config = WorkflowGuardConfig::new().with_max_fan_out(3);
        root.enter_wave(&config, "child", 2).expect("room for two");
        let mut left = root.branch_into("child");
        let mut right = root.branch_into("child");

        assert_eq!(left.enter(&config, "leaf"), Ok(()));
        assert_eq!(
            right.enter(&config, "leaf"),
            Err(WorkflowRejection::MaxFanOutExceeded {
                invocations: 3,
                max: 3
            })
        );
        assert_eq!(right.depth(), 1, "a refused invocation records nothing");
        assert_eq!(right.call_chain(), ["child"]);
    }
}
