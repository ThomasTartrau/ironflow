//! Cost guardrails: per-run cost cap and global monthly quota.
//!
//! `max_budget_usd` on an [`AgentConfig`](ironflow_core::provider::AgentConfig)
//! only caps a single agent invocation. A workflow with a loop, a dynamic
//! `ctx.parallel()`, or a chain of sub-workflows can chain dozens of agent
//! calls with no cumulative bound. [`BudgetConfig`] closes that gap:
//!
//! - **Per-run cap** -- checked before every agent step. Crossing it cancels
//!   the run *before* the step is launched, so no spend happens.
//! - **Monthly quota** -- checked when a new run is created. Crossing it
//!   refuses the creation; runs already in flight are untouched.
//!
//! # Examples
//!
//! ```
//! use ironflow_engine::budget::BudgetConfig;
//! use rust_decimal::Decimal;
//!
//! let config = BudgetConfig::new()
//!     .default_run_max_cost_usd(Decimal::new(500, 2))
//!     .monthly_cost_limit_usd(Decimal::new(20000, 2));
//!
//! assert_eq!(config.default_run_max_cost_usd, Some(Decimal::new(500, 2)));
//! ```

use std::env;
use std::str::FromStr;

use chrono::{DateTime, Datelike, TimeZone, Utc};
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;
use tracing::warn;

/// Environment variable holding the server-wide default per-run cost cap.
pub const DEFAULT_RUN_MAX_COST_ENV: &str = "IRONFLOW_DEFAULT_RUN_MAX_COST_USD";

/// Environment variable holding the global monthly cost quota.
pub const MONTHLY_COST_LIMIT_ENV: &str = "IRONFLOW_MONTHLY_COST_LIMIT_USD";

/// Server-level cost guardrails.
///
/// Both fields default to `None`, which disables the corresponding check and
/// preserves the pre-existing behaviour exactly.
///
/// # Examples
///
/// ```
/// use ironflow_engine::budget::BudgetConfig;
///
/// let config = BudgetConfig::new();
/// assert!(config.default_run_max_cost_usd.is_none());
/// assert!(config.monthly_cost_limit_usd.is_none());
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BudgetConfig {
    /// Default cost cap applied to a run when neither the creation request nor
    /// the handler declares one. `None` means no default cap.
    pub default_run_max_cost_usd: Option<Decimal>,
    /// Global quota for the current calendar month (UTC). `None` disables the
    /// monthly check.
    pub monthly_cost_limit_usd: Option<Decimal>,
}

impl BudgetConfig {
    /// Create a configuration with both guardrails disabled.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::budget::BudgetConfig;
    ///
    /// assert_eq!(BudgetConfig::new(), BudgetConfig::default());
    /// ```
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the server-wide default per-run cost cap.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::budget::BudgetConfig;
    /// use rust_decimal::Decimal;
    ///
    /// let config = BudgetConfig::new().default_run_max_cost_usd(Decimal::new(150, 2));
    /// assert_eq!(config.default_run_max_cost_usd, Some(Decimal::new(150, 2)));
    /// ```
    pub fn default_run_max_cost_usd(mut self, cap: Decimal) -> Self {
        self.default_run_max_cost_usd = Some(cap);
        self
    }

    /// Set the global monthly cost quota.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::budget::BudgetConfig;
    /// use rust_decimal::Decimal;
    ///
    /// let config = BudgetConfig::new().monthly_cost_limit_usd(Decimal::new(10000, 2));
    /// assert_eq!(config.monthly_cost_limit_usd, Some(Decimal::new(10000, 2)));
    /// ```
    pub fn monthly_cost_limit_usd(mut self, limit: Decimal) -> Self {
        self.monthly_cost_limit_usd = Some(limit);
        self
    }

    /// Load the configuration from the environment.
    ///
    /// Reads [`DEFAULT_RUN_MAX_COST_ENV`] and [`MONTHLY_COST_LIMIT_ENV`]. A
    /// variable that is unset, empty, unparseable, or negative is ignored (the
    /// corresponding guardrail stays disabled) and logged at `WARN`. Loading
    /// never fails, so a malformed value can never prevent the server from
    /// starting.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::budget::BudgetConfig;
    ///
    /// let config = BudgetConfig::from_env();
    /// # let _ = config;
    /// ```
    pub fn from_env() -> Self {
        Self {
            default_run_max_cost_usd: read_decimal_env(DEFAULT_RUN_MAX_COST_ENV),
            monthly_cost_limit_usd: read_decimal_env(MONTHLY_COST_LIMIT_ENV),
        }
    }

    /// Resolve the cost cap of a run being created.
    ///
    /// Priority, strongest first: the value supplied at creation time, then the
    /// handler default, then the server default. `None` at every level means
    /// the run has no cap.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::budget::BudgetConfig;
    /// use rust_decimal::Decimal;
    ///
    /// let server = Decimal::new(100, 2);
    /// let handler = Decimal::new(200, 2);
    /// let requested = Decimal::new(300, 2);
    /// let config = BudgetConfig::new().default_run_max_cost_usd(server);
    ///
    /// assert_eq!(config.resolve_run_cap(Some(requested), Some(handler)), Some(requested));
    /// assert_eq!(config.resolve_run_cap(None, Some(handler)), Some(handler));
    /// assert_eq!(config.resolve_run_cap(None, None), Some(server));
    /// ```
    pub fn resolve_run_cap(
        &self,
        requested: Option<Decimal>,
        handler_default: Option<Decimal>,
    ) -> Option<Decimal> {
        requested
            .or(handler_default)
            .or(self.default_run_max_cost_usd)
    }
}

/// Read a non-negative [`Decimal`] from an environment variable.
///
/// Returns `None` when the variable is unset, empty, unparseable, or negative.
fn read_decimal_env(name: &str) -> Option<Decimal> {
    let raw = env::var(name).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    match Decimal::from_str(trimmed) {
        Ok(value) if value >= Decimal::ZERO => Some(value),
        Ok(value) => {
            warn!(env = name, value = %value, "ignoring negative budget limit");
            None
        }
        Err(e) => {
            warn!(env = name, value = trimmed, error = %e, "ignoring unparseable budget limit");
            None
        }
    }
}

/// Convert an agent step budget expressed as `f64` into a [`Decimal`].
///
/// A missing budget counts as zero: the run cap check then reduces to
/// "has the run already spent more than its cap?". A `NaN` or infinite value
/// also counts as zero rather than poisoning the comparison.
///
/// # Examples
///
/// ```
/// use ironflow_engine::budget::step_budget_usd;
/// use rust_decimal::Decimal;
///
/// assert_eq!(step_budget_usd(Some(0.25)), Decimal::new(25, 2));
/// assert_eq!(step_budget_usd(None), Decimal::ZERO);
/// assert_eq!(step_budget_usd(Some(f64::NAN)), Decimal::ZERO);
/// ```
pub fn step_budget_usd(max_budget_usd: Option<f64>) -> Decimal {
    max_budget_usd
        .and_then(Decimal::from_f64)
        .unwrap_or(Decimal::ZERO)
}

/// Start of the current calendar month, at 00:00:00 UTC.
///
/// Used as the lower bound of the monthly quota window.
///
/// # Examples
///
/// ```
/// use chrono::{Datelike, TimeZone, Timelike, Utc};
/// use ironflow_engine::budget::month_start;
///
/// let start = month_start(Utc.with_ymd_and_hms(2026, 7, 26, 15, 30, 0).unwrap());
/// assert_eq!(start.year(), 2026);
/// assert_eq!(start.month(), 7);
/// assert_eq!(start.day(), 1);
/// assert_eq!(start.hour(), 0);
/// ```
///
/// # Panics
///
/// Never panics for a valid [`DateTime<Utc>`]: day 1 at midnight always exists
/// for any year/month pair reachable from an existing timestamp.
pub fn month_start(now: DateTime<Utc>) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(now.year(), now.month(), 1, 0, 0, 0)
        .single()
        .expect("first day of month at midnight UTC is always unambiguous")
}

#[cfg(test)]
mod tests {
    use chrono::Timelike;

    use super::*;

    #[test]
    fn new_disables_both_guardrails() {
        let config = BudgetConfig::new();
        assert!(config.default_run_max_cost_usd.is_none());
        assert!(config.monthly_cost_limit_usd.is_none());
    }

    #[test]
    fn resolve_run_cap_prefers_request_then_handler_then_server() {
        let config = BudgetConfig::new().default_run_max_cost_usd(Decimal::ONE);

        assert_eq!(
            config.resolve_run_cap(Some(Decimal::TEN), Some(Decimal::TWO)),
            Some(Decimal::TEN)
        );
        assert_eq!(
            config.resolve_run_cap(None, Some(Decimal::TWO)),
            Some(Decimal::TWO)
        );
        assert_eq!(config.resolve_run_cap(None, None), Some(Decimal::ONE));
    }

    #[test]
    fn resolve_run_cap_without_server_default_is_none() {
        let config = BudgetConfig::new();
        assert_eq!(config.resolve_run_cap(None, None), None);
    }

    #[test]
    fn resolve_run_cap_accepts_explicit_zero() {
        let config = BudgetConfig::new().default_run_max_cost_usd(Decimal::TEN);
        assert_eq!(
            config.resolve_run_cap(Some(Decimal::ZERO), None),
            Some(Decimal::ZERO)
        );
    }

    #[test]
    fn step_budget_usd_maps_missing_and_invalid_to_zero() {
        assert_eq!(step_budget_usd(None), Decimal::ZERO);
        assert_eq!(step_budget_usd(Some(f64::NAN)), Decimal::ZERO);
        assert_eq!(step_budget_usd(Some(f64::INFINITY)), Decimal::ZERO);
        assert_eq!(step_budget_usd(Some(f64::NEG_INFINITY)), Decimal::ZERO);
    }

    #[test]
    fn step_budget_usd_converts_finite_values() {
        assert_eq!(step_budget_usd(Some(0.0)), Decimal::ZERO);
        assert_eq!(step_budget_usd(Some(0.25)), Decimal::new(25, 2));
        assert_eq!(step_budget_usd(Some(1.5)), Decimal::new(15, 1));
    }

    #[test]
    fn month_start_truncates_to_first_day_midnight() {
        let now = Utc.with_ymd_and_hms(2026, 2, 17, 23, 59, 59).unwrap();
        let start = month_start(now);

        assert_eq!(start.year(), 2026);
        assert_eq!(start.month(), 2);
        assert_eq!(start.day(), 1);
        assert_eq!(start.hour(), 0);
        assert_eq!(start.minute(), 0);
        assert_eq!(start.second(), 0);
    }

    #[test]
    fn month_start_is_idempotent() {
        let now = Utc.with_ymd_and_hms(2026, 12, 1, 0, 0, 0).unwrap();
        assert_eq!(month_start(month_start(now)), month_start(now));
    }

    #[test]
    fn read_decimal_env_rejects_unset_empty_negative_and_garbage() {
        // SAFETY: single-threaded test, variables are scoped to this test name.
        unsafe {
            env::remove_var("IRONFLOW_TEST_BUDGET_UNSET");
            env::set_var("IRONFLOW_TEST_BUDGET_EMPTY", "   ");
            env::set_var("IRONFLOW_TEST_BUDGET_NEGATIVE", "-1.5");
            env::set_var("IRONFLOW_TEST_BUDGET_GARBAGE", "five dollars");
            env::set_var("IRONFLOW_TEST_BUDGET_VALID", " 2.50 ");
        }

        assert_eq!(read_decimal_env("IRONFLOW_TEST_BUDGET_UNSET"), None);
        assert_eq!(read_decimal_env("IRONFLOW_TEST_BUDGET_EMPTY"), None);
        assert_eq!(read_decimal_env("IRONFLOW_TEST_BUDGET_NEGATIVE"), None);
        assert_eq!(read_decimal_env("IRONFLOW_TEST_BUDGET_GARBAGE"), None);
        assert_eq!(
            read_decimal_env("IRONFLOW_TEST_BUDGET_VALID"),
            Some(Decimal::new(250, 2))
        );

        // SAFETY: single-threaded test cleanup.
        unsafe {
            env::remove_var("IRONFLOW_TEST_BUDGET_EMPTY");
            env::remove_var("IRONFLOW_TEST_BUDGET_NEGATIVE");
            env::remove_var("IRONFLOW_TEST_BUDGET_GARBAGE");
            env::remove_var("IRONFLOW_TEST_BUDGET_VALID");
        }
    }

    #[test]
    fn read_decimal_env_accepts_zero() {
        // SAFETY: single-threaded test.
        unsafe { env::set_var("IRONFLOW_TEST_BUDGET_ZERO", "0") };
        assert_eq!(
            read_decimal_env("IRONFLOW_TEST_BUDGET_ZERO"),
            Some(Decimal::ZERO)
        );
        // SAFETY: single-threaded test cleanup.
        unsafe { env::remove_var("IRONFLOW_TEST_BUDGET_ZERO") };
    }
}
