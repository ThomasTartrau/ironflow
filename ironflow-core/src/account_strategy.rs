//! Selection of a Provider Account for an agent step.
//!
//! [`select_account`] filters out unavailable accounts (an applicable window
//! exhausted, or `max_concurrency` reached), then hands the rest to an
//! [`AccountStrategy`]. Three strategies ship: [`LeastUtilized`] (the
//! default), [`Priority`] and [`RoundRobin`].
//!
//! # Examples
//!
//! ```
//! use chrono::Utc;
//! use ironflow_core::account_strategy::{AccountCandidate, LeastUtilized, select_account};
//!
//! let candidates = vec![AccountCandidate {
//!     id: "1".to_string(),
//!     name: "perso".to_string(),
//!     priority: 10,
//!     max_concurrency: None,
//!     running_steps: 0,
//!     windows: Vec::new(),
//! }];
//! let chosen = select_account(&LeastUtilized, &candidates, "claude-sonnet-4-5", Utc::now());
//! assert_eq!(chosen.map(|c| c.name.as_str()), Some("perso"));
//! ```

use std::cmp::Ordering;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

use chrono::{DateTime, Utc};

use crate::account::AccountWindow;

/// Name of the default strategy.
pub const DEFAULT_STRATEGY: &str = "least_utilized";

/// Weight of one running step in the [`LeastUtilized`] score.
const RUNNING_STEP_PENALTY: f64 = 0.15;

/// An account a step may run under, with its current usage.
///
/// # Examples
///
/// See the [module documentation](self).
#[derive(Debug, Clone, PartialEq)]
pub struct AccountCandidate {
    /// Account identifier.
    pub id: String,
    /// Account name (unique slug).
    pub name: String,
    /// Priority: lower values are preferred.
    pub priority: i32,
    /// Maximum concurrent steps, `None` for unlimited.
    pub max_concurrency: Option<u32>,
    /// Steps currently running under the account.
    pub running_steps: u32,
    /// Last observed windows.
    pub windows: Vec<AccountWindow>,
}

impl AccountCandidate {
    /// Whether the account can take a step for `model` at `now`.
    ///
    /// # Examples
    ///
    /// ```
    /// use chrono::Utc;
    /// use ironflow_core::account_strategy::AccountCandidate;
    ///
    /// let full = AccountCandidate {
    ///     id: "1".to_string(),
    ///     name: "team".to_string(),
    ///     priority: 10,
    ///     max_concurrency: Some(1),
    ///     running_steps: 1,
    ///     windows: Vec::new(),
    /// };
    /// assert!(!full.is_available("claude-sonnet-4-5", Utc::now()));
    /// ```
    pub fn is_available(&self, model: &str, now: DateTime<Utc>) -> bool {
        let exhausted = self
            .windows
            .iter()
            .any(|w| w.applies_to(model) && w.is_exhausted(now));
        let saturated = self
            .max_concurrency
            .is_some_and(|max| self.running_steps >= max);
        !exhausted && !saturated
    }

    fn utilization_for(&self, model: &str, now: DateTime<Utc>) -> f64 {
        self.windows
            .iter()
            .filter(|w| w.applies_to(model))
            .map(|w| w.effective_utilization(now))
            .fold(0.0, f64::max)
    }
}

/// What a strategy chooses among.
///
/// # Examples
///
/// ```
/// use chrono::Utc;
/// use ironflow_core::account_strategy::{AccountStrategy, Priority, SelectionRequest};
///
/// let request = SelectionRequest { candidates: &[], model: "m", now: Utc::now() };
/// assert!(Priority.select(&request).is_none());
/// ```
#[derive(Debug, Clone, Copy)]
pub struct SelectionRequest<'a> {
    /// Available candidates only.
    pub candidates: &'a [AccountCandidate],
    /// Model the step requests.
    pub model: &'a str,
    /// Reference time.
    pub now: DateTime<Utc>,
}

/// A policy picking one account among available candidates.
///
/// # Examples
///
/// ```
/// use ironflow_core::account_strategy::{AccountStrategy, RoundRobin};
///
/// assert_eq!(RoundRobin::default().name(), "round_robin");
/// ```
pub trait AccountStrategy: Send + Sync {
    /// Strategy name, as accepted by [`strategy_by_name`].
    fn name(&self) -> &str;

    /// Pick one candidate, `None` when there is none.
    fn select<'a>(&self, request: &SelectionRequest<'a>) -> Option<&'a AccountCandidate>;
}

/// Filter the available candidates, then let `strategy` pick one.
///
/// # Examples
///
/// See the [module documentation](self).
pub fn select_account<'a>(
    strategy: &dyn AccountStrategy,
    candidates: &'a [AccountCandidate],
    model: &str,
    now: DateTime<Utc>,
) -> Option<&'a AccountCandidate> {
    let available: Vec<AccountCandidate> = candidates
        .iter()
        .filter(|c| c.is_available(model, now))
        .cloned()
        .collect();
    let request = SelectionRequest {
        candidates: &available,
        model,
        now,
    };
    let chosen_id = strategy.select(&request)?.id.clone();
    candidates.iter().find(|c| c.id == chosen_id)
}

fn by_priority_then_name(a: &AccountCandidate, b: &AccountCandidate) -> Ordering {
    a.priority
        .cmp(&b.priority)
        .then_with(|| a.name.cmp(&b.name))
}

/// Picks the account with the most headroom.
///
/// Score = highest effective utilization of the applicable windows plus
/// `0.15` per running step; the lowest score wins, ties go to the lower
/// priority value, then the name.
///
/// # Examples
///
/// ```
/// use ironflow_core::account_strategy::{AccountStrategy, LeastUtilized};
///
/// assert_eq!(LeastUtilized.name(), "least_utilized");
/// ```
#[derive(Debug, Clone, Copy, Default)]
pub struct LeastUtilized;

impl AccountStrategy for LeastUtilized {
    fn name(&self) -> &str {
        "least_utilized"
    }

    fn select<'a>(&self, request: &SelectionRequest<'a>) -> Option<&'a AccountCandidate> {
        let score = |c: &AccountCandidate| {
            c.utilization_for(request.model, request.now)
                + RUNNING_STEP_PENALTY * f64::from(c.running_steps)
        };
        request.candidates.iter().min_by(|a, b| {
            score(a)
                .total_cmp(&score(b))
                .then_with(|| by_priority_then_name(a, b))
        })
    }
}

/// Picks the lowest priority value, ties by name.
///
/// # Examples
///
/// ```
/// use ironflow_core::account_strategy::{AccountStrategy, Priority};
///
/// assert_eq!(Priority.name(), "priority");
/// ```
#[derive(Debug, Clone, Copy, Default)]
pub struct Priority;

impl AccountStrategy for Priority {
    fn name(&self) -> &str {
        "priority"
    }

    fn select<'a>(&self, request: &SelectionRequest<'a>) -> Option<&'a AccountCandidate> {
        request
            .candidates
            .iter()
            .min_by(|a, b| by_priority_then_name(a, b))
    }
}

/// Cycles through the candidates sorted by name.
///
/// # Examples
///
/// ```
/// use chrono::Utc;
/// use ironflow_core::account_strategy::{AccountCandidate, AccountStrategy, RoundRobin, SelectionRequest};
///
/// let candidate = |name: &str| AccountCandidate {
///     id: name.to_string(),
///     name: name.to_string(),
///     priority: 0,
///     max_concurrency: None,
///     running_steps: 0,
///     windows: Vec::new(),
/// };
/// let candidates = [candidate("a"), candidate("b")];
/// let strategy = RoundRobin::default();
/// let request = SelectionRequest { candidates: &candidates, model: "m", now: Utc::now() };
/// assert_eq!(strategy.select(&request).map(|c| c.name.as_str()), Some("a"));
/// assert_eq!(strategy.select(&request).map(|c| c.name.as_str()), Some("b"));
/// ```
#[derive(Debug, Default)]
pub struct RoundRobin {
    cursor: AtomicUsize,
}

impl AccountStrategy for RoundRobin {
    fn name(&self) -> &str {
        "round_robin"
    }

    fn select<'a>(&self, request: &SelectionRequest<'a>) -> Option<&'a AccountCandidate> {
        if request.candidates.is_empty() {
            return None;
        }
        let mut sorted: Vec<&'a AccountCandidate> = request.candidates.iter().collect();
        sorted.sort_by(|a, b| a.name.cmp(&b.name));
        let index = self.cursor.fetch_add(1, AtomicOrdering::Relaxed) % sorted.len();
        Some(sorted[index])
    }
}

/// Build a strategy from its name, `None` when unknown.
///
/// # Examples
///
/// ```
/// use ironflow_core::account_strategy::strategy_by_name;
///
/// assert_eq!(strategy_by_name("priority").map(|s| s.name().to_string()), Some("priority".to_string()));
/// assert!(strategy_by_name("random").is_none());
/// ```
pub fn strategy_by_name(name: &str) -> Option<Arc<dyn AccountStrategy>> {
    match name {
        "least_utilized" => Some(Arc::new(LeastUtilized)),
        "priority" => Some(Arc::new(Priority)),
        "round_robin" => Some(Arc::new(RoundRobin::default())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::WindowStatus;
    use chrono::TimeDelta;

    fn window(name: &str, utilization: f64, status: WindowStatus) -> AccountWindow {
        AccountWindow {
            window: name.to_string(),
            utilization,
            resets_at: Some(Utc::now() + TimeDelta::hours(1)),
            status,
            model_scope: None,
            observed_at: Utc::now(),
        }
    }

    fn candidate(name: &str, priority: i32, windows: Vec<AccountWindow>) -> AccountCandidate {
        AccountCandidate {
            id: format!("id-{name}"),
            name: name.to_string(),
            priority,
            max_concurrency: None,
            running_steps: 0,
            windows,
        }
    }

    fn pick<'a>(
        strategy: &dyn AccountStrategy,
        candidates: &'a [AccountCandidate],
        model: &str,
    ) -> Option<&'a str> {
        select_account(strategy, candidates, model, Utc::now()).map(|c| c.name.as_str())
    }

    #[test]
    fn least_utilized_picks_lowest_utilization() {
        let candidates = [
            candidate(
                "a",
                1,
                vec![window("five_hour", 0.7, WindowStatus::Allowed)],
            ),
            candidate(
                "b",
                2,
                vec![window("five_hour", 0.2, WindowStatus::Allowed)],
            ),
        ];
        assert_eq!(pick(&LeastUtilized, &candidates, "sonnet"), Some("b"));
    }

    #[test]
    fn least_utilized_penalises_running_steps() {
        let mut busy = candidate(
            "a",
            1,
            vec![window("five_hour", 0.1, WindowStatus::Allowed)],
        );
        busy.running_steps = 2;
        let idle = candidate(
            "b",
            2,
            vec![window("five_hour", 0.3, WindowStatus::Allowed)],
        );
        let candidates = [busy, idle];
        assert_eq!(pick(&LeastUtilized, &candidates, "sonnet"), Some("b"));
    }

    #[test]
    fn least_utilized_ignores_windows_already_reset() {
        let mut reset = window("five_hour", 0.95, WindowStatus::Allowed);
        reset.resets_at = Some(Utc::now() - TimeDelta::minutes(5));
        let candidates = [
            candidate("a", 1, vec![reset]),
            candidate(
                "b",
                2,
                vec![window("five_hour", 0.3, WindowStatus::Allowed)],
            ),
        ];
        assert_eq!(pick(&LeastUtilized, &candidates, "sonnet"), Some("a"));
    }

    #[test]
    fn least_utilized_ties_by_priority_then_name() {
        let candidates = [
            candidate("c", 5, Vec::new()),
            candidate("b", 1, Vec::new()),
            candidate("a", 1, Vec::new()),
        ];
        assert_eq!(pick(&LeastUtilized, &candidates, "sonnet"), Some("a"));
    }

    #[test]
    fn priority_picks_lowest_value_and_ties_by_name() {
        let candidates = [
            candidate(
                "z",
                1,
                vec![window("five_hour", 0.9, WindowStatus::Allowed)],
            ),
            candidate("y", 1, Vec::new()),
            candidate("a", 5, Vec::new()),
        ];
        assert_eq!(pick(&Priority, &candidates, "sonnet"), Some("y"));
    }

    #[test]
    fn round_robin_cycles_through_candidates() {
        let candidates = [
            candidate("b", 1, Vec::new()),
            candidate("a", 1, Vec::new()),
            candidate("c", 1, Vec::new()),
        ];
        let strategy = RoundRobin::default();
        let picks: Vec<_> = (0..4)
            .map(|_| pick(&strategy, &candidates, "sonnet").unwrap_or_default())
            .collect();
        assert_eq!(picks, vec!["a", "b", "c", "a"]);
    }

    #[test]
    fn select_account_skips_exhausted_accounts() {
        let candidates = [
            candidate(
                "a",
                1,
                vec![window("five_hour", 1.0, WindowStatus::Rejected)],
            ),
            candidate(
                "b",
                2,
                vec![window("five_hour", 0.9, WindowStatus::Allowed)],
            ),
        ];
        assert_eq!(pick(&Priority, &candidates, "sonnet"), Some("b"));
    }

    #[test]
    fn select_account_reuses_rejected_window_after_reset() {
        let mut rejected = window("five_hour", 1.0, WindowStatus::Rejected);
        rejected.resets_at = Some(Utc::now() - TimeDelta::seconds(1));
        let candidates = [candidate("a", 1, vec![rejected])];
        assert_eq!(pick(&Priority, &candidates, "sonnet"), Some("a"));
    }

    #[test]
    fn select_account_respects_max_concurrency() {
        let mut full = candidate("a", 1, Vec::new());
        full.max_concurrency = Some(2);
        full.running_steps = 2;
        let mut open = candidate("b", 2, Vec::new());
        open.max_concurrency = Some(2);
        open.running_steps = 1;
        let candidates = [full, open];
        assert_eq!(pick(&Priority, &candidates, "sonnet"), Some("b"));
    }

    #[test]
    fn select_account_model_scoped_rejection_blocks_only_matching_model() {
        let mut opus = window("seven_day", 1.0, WindowStatus::Rejected);
        opus.model_scope = Some("opus".to_string());
        let candidates = [candidate("a", 1, vec![opus])];
        assert_eq!(pick(&Priority, &candidates, "claude-opus-4-1"), None);
        assert_eq!(pick(&Priority, &candidates, "claude-sonnet-4-5"), Some("a"));
    }

    #[test]
    fn select_account_empty_or_all_exhausted_returns_none() {
        assert_eq!(pick(&LeastUtilized, &[], "sonnet"), None);
        assert_eq!(pick(&RoundRobin::default(), &[], "sonnet"), None);
        let candidates = [
            candidate(
                "a",
                1,
                vec![window("five_hour", 1.0, WindowStatus::Rejected)],
            ),
            candidate(
                "b",
                1,
                vec![window("seven_day", 1.0, WindowStatus::Rejected)],
            ),
        ];
        assert_eq!(pick(&LeastUtilized, &candidates, "sonnet"), None);
    }

    #[test]
    fn strategy_by_name_known_and_unknown() {
        for name in ["least_utilized", "priority", "round_robin"] {
            let strategy = strategy_by_name(name).expect("known strategy");
            assert_eq!(strategy.name(), name);
        }
        assert!(strategy_by_name("random").is_none());
        assert!(strategy_by_name("").is_none());
        assert!(strategy_by_name(DEFAULT_STRATEGY).is_some());
    }
}
