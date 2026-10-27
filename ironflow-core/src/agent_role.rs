//! Identity and cost cap of an agent role.
//!
//! An agent role (see `ironflow_engine::config::AgentRole`) may declare a
//! name and a [`RoleCostCap`]. The role's tasks carry an [`AgentRoleBinding`]
//! on their [`AgentConfig`](crate::provider::AgentConfig), which the engine
//! reads to charge every agent step of the role to one shared counter over a
//! calendar [`CostPeriod`] (UTC).
//!
//! # Examples
//!
//! ```
//! use chrono::{TimeZone, Utc};
//! use ironflow_core::agent_role::{AgentRoleBinding, CostPeriod, RoleCostCap};
//! use rust_decimal::Decimal;
//!
//! let binding = AgentRoleBinding::new("reviewer", Some(RoleCostCap::per_month(Decimal::TEN)));
//! assert_eq!(binding.period, CostPeriod::Month);
//!
//! let now = Utc.with_ymd_and_hms(2026, 7, 26, 15, 30, 0).unwrap();
//! assert_eq!(CostPeriod::Month.start(now), Utc.with_ymd_and_hms(2026, 7, 1, 0, 0, 0).unwrap());
//! ```

use chrono::{DateTime, Datelike, Days, Months, TimeZone, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use strum::{Display, EnumString, IntoStaticStr};

/// Longest name an agent role may have, in bytes.
pub const MAX_ROLE_NAME_LEN: usize = 64;

/// Whether `name` is a valid agent role name.
///
/// A name has 1 to [`MAX_ROLE_NAME_LEN`] bytes of ASCII lowercase letters,
/// digits, `-`, `_` and `.`, and starts with a letter or a digit. It is used
/// in URLs and as a database key, so nothing else is accepted.
///
/// # Examples
///
/// ```
/// use ironflow_core::agent_role::is_valid_role_name;
///
/// assert!(is_valid_role_name("code-reviewer.v2"));
/// assert!(!is_valid_role_name("Reviewer"));
/// assert!(!is_valid_role_name("-reviewer"));
/// assert!(!is_valid_role_name(""));
/// ```
pub fn is_valid_role_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    let alphanumeric = |b: &u8| b.is_ascii_lowercase() || b.is_ascii_digit();
    bytes.first().is_some_and(alphanumeric)
        && bytes.len() <= MAX_ROLE_NAME_LEN
        && bytes
            .iter()
            .all(|b| alphanumeric(b) || matches!(b, b'-' | b'_' | b'.'))
}

/// Calendar period, in UTC, over which the cost of a role is counted.
///
/// # Examples
///
/// ```
/// use std::str::FromStr;
/// use ironflow_core::agent_role::CostPeriod;
///
/// assert_eq!(CostPeriod::from_str("day").ok(), Some(CostPeriod::Day));
/// assert_eq!(CostPeriod::Month.to_string(), "month");
/// ```
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Display,
    EnumString,
    IntoStaticStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum CostPeriod {
    /// From midnight UTC to the next midnight UTC.
    Day,
    /// From the 1st of the month at 00:00 UTC to the 1st of the next month.
    Month,
}

impl CostPeriod {
    /// Stable name of the period, as stored and sent over the API.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::agent_role::CostPeriod;
    ///
    /// assert_eq!(CostPeriod::Day.as_str(), "day");
    /// ```
    pub fn as_str(self) -> &'static str {
        self.into()
    }

    /// Start of the period containing `now`.
    ///
    /// # Examples
    ///
    /// ```
    /// use chrono::{TimeZone, Utc};
    /// use ironflow_core::agent_role::CostPeriod;
    ///
    /// let now = Utc.with_ymd_and_hms(2026, 7, 26, 15, 30, 0).unwrap();
    /// assert_eq!(CostPeriod::Day.start(now), Utc.with_ymd_and_hms(2026, 7, 26, 0, 0, 0).unwrap());
    /// ```
    ///
    /// # Panics
    ///
    /// Never panics for a valid [`DateTime<Utc>`]: midnight of an existing day
    /// is always unambiguous in UTC.
    pub fn start(self, now: DateTime<Utc>) -> DateTime<Utc> {
        let day = match self {
            Self::Day => now.day(),
            Self::Month => 1,
        };
        Utc.with_ymd_and_hms(now.year(), now.month(), day, 0, 0, 0)
            .single()
            .expect("midnight UTC of an existing day is always unambiguous")
    }

    /// Start of the period following the one containing `now`: when the
    /// counter resets.
    ///
    /// # Examples
    ///
    /// ```
    /// use chrono::{TimeZone, Utc};
    /// use ironflow_core::agent_role::CostPeriod;
    ///
    /// let now = Utc.with_ymd_and_hms(2026, 12, 26, 15, 30, 0).unwrap();
    /// assert_eq!(
    ///     CostPeriod::Month.next_start(now),
    ///     Utc.with_ymd_and_hms(2027, 1, 1, 0, 0, 0).unwrap()
    /// );
    /// ```
    ///
    /// # Panics
    ///
    /// Panics only past the last date chrono represents (year 262143).
    pub fn next_start(self, now: DateTime<Utc>) -> DateTime<Utc> {
        let start = self.start(now);
        match self {
            Self::Day => start
                .checked_add_days(Days::new(1))
                .expect("the day after a representable date is representable"),
            Self::Month => start
                .checked_add_months(Months::new(1))
                .expect("the month after a representable date is representable"),
        }
    }
}

/// Cost cap of an agent role over a calendar period.
///
/// # Examples
///
/// ```
/// use ironflow_core::agent_role::{CostPeriod, RoleCostCap};
/// use rust_decimal::Decimal;
///
/// let cap = RoleCostCap::per_month(Decimal::new(50, 0));
/// assert_eq!(cap.limit_usd(), Decimal::new(50, 0));
/// assert_eq!(cap.period(), CostPeriod::Month);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoleCostCap {
    limit_usd: Decimal,
    period: CostPeriod,
}

impl RoleCostCap {
    /// Cap of `limit_usd` per calendar day (UTC). Zero disables the role.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::agent_role::{CostPeriod, RoleCostCap};
    /// use rust_decimal::Decimal;
    ///
    /// assert_eq!(RoleCostCap::per_day(Decimal::ONE).period(), CostPeriod::Day);
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if `limit_usd` is negative.
    pub fn per_day(limit_usd: Decimal) -> Self {
        Self::new(limit_usd, CostPeriod::Day)
    }

    /// Cap of `limit_usd` per calendar month (UTC). Zero disables the role.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::agent_role::{CostPeriod, RoleCostCap};
    /// use rust_decimal::Decimal;
    ///
    /// assert_eq!(RoleCostCap::per_month(Decimal::ONE).period(), CostPeriod::Month);
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if `limit_usd` is negative.
    pub fn per_month(limit_usd: Decimal) -> Self {
        Self::new(limit_usd, CostPeriod::Month)
    }

    fn new(limit_usd: Decimal, period: CostPeriod) -> Self {
        assert!(
            limit_usd >= Decimal::ZERO,
            "the cost cap of an agent role must not be negative, got {limit_usd}"
        );
        Self { limit_usd, period }
    }

    /// The cap, in USD.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::agent_role::RoleCostCap;
    /// use rust_decimal::Decimal;
    ///
    /// assert_eq!(RoleCostCap::per_day(Decimal::TWO).limit_usd(), Decimal::TWO);
    /// ```
    pub fn limit_usd(&self) -> Decimal {
        self.limit_usd
    }

    /// The period the cap applies to.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::agent_role::{CostPeriod, RoleCostCap};
    /// use rust_decimal::Decimal;
    ///
    /// assert_eq!(RoleCostCap::per_day(Decimal::TWO).period(), CostPeriod::Day);
    /// ```
    pub fn period(&self) -> CostPeriod {
        self.period
    }
}

/// Agent role an invocation is charged to, attached to its
/// [`AgentConfig`](crate::provider::AgentConfig).
///
/// # Examples
///
/// ```
/// use ironflow_core::agent_role::{AgentRoleBinding, CostPeriod};
///
/// let binding = AgentRoleBinding::new("reviewer", None);
/// assert_eq!(binding.name, "reviewer");
/// assert_eq!(binding.cost_cap_usd, None);
/// assert_eq!(binding.period, CostPeriod::Month);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRoleBinding {
    /// Name of the role, valid per [`is_valid_role_name`].
    pub name: String,
    /// Cap declared in code, `None` when the role only counts its cost.
    pub cost_cap_usd: Option<Decimal>,
    /// Period of the counter; [`CostPeriod::Month`] when no cap is declared.
    pub period: CostPeriod,
}

impl AgentRoleBinding {
    /// Bind to the role `name`, with its optional code cap.
    ///
    /// A role without a code cap still counts its cost over a month, so an
    /// operator cap set later applies to what was already spent.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::agent_role::{AgentRoleBinding, CostPeriod, RoleCostCap};
    /// use rust_decimal::Decimal;
    ///
    /// let binding = AgentRoleBinding::new("triage", Some(RoleCostCap::per_day(Decimal::ONE)));
    /// assert_eq!(binding.cost_cap_usd, Some(Decimal::ONE));
    /// assert_eq!(binding.period, CostPeriod::Day);
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if `name` is not valid per [`is_valid_role_name`].
    pub fn new(name: &str, cap: Option<RoleCostCap>) -> Self {
        assert!(
            is_valid_role_name(name),
            "invalid agent role name {name:?}: 1 to {MAX_ROLE_NAME_LEN} ASCII lowercase letters, digits, '-', '_' or '.', starting with a letter or a digit"
        );
        Self {
            name: name.to_string(),
            cost_cap_usd: cap.map(|c| c.limit_usd()),
            period: cap.map(|c| c.period()).unwrap_or(CostPeriod::Month),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use serde_json::to_string;

    use super::*;

    fn at(y: i32, m: u32, d: u32, h: u32, min: u32, s: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, min, s).unwrap()
    }

    #[test]
    fn role_names_are_validated() {
        assert!(is_valid_role_name("code-reviewer.v2"));
        assert!(is_valid_role_name("a"));
        assert!(is_valid_role_name("9_lives"));
        assert!(is_valid_role_name(&"a".repeat(64)));
        assert!(!is_valid_role_name(&"a".repeat(65)));
        assert!(!is_valid_role_name(""));
        assert!(!is_valid_role_name("Reviewer"));
        assert!(!is_valid_role_name("revi\u{e9}wer"));
        assert!(!is_valid_role_name("-reviewer"));
        assert!(!is_valid_role_name(".reviewer"));
        assert!(!is_valid_role_name("a b"));
        assert!(!is_valid_role_name("a/b"));
    }

    #[test]
    fn day_period_starts_at_midnight_utc() {
        let day = CostPeriod::Day;
        assert_eq!(
            day.start(at(2026, 7, 26, 15, 30, 0)),
            at(2026, 7, 26, 0, 0, 0)
        );
        assert_eq!(
            day.start(at(2026, 7, 26, 0, 0, 0)),
            at(2026, 7, 26, 0, 0, 0)
        );
        assert_eq!(
            day.next_start(at(2026, 12, 31, 23, 59, 59)),
            at(2027, 1, 1, 0, 0, 0)
        );
        assert_eq!(
            day.next_start(at(2028, 2, 28, 12, 0, 0)),
            at(2028, 2, 29, 0, 0, 0)
        );
    }

    #[test]
    fn month_period_starts_on_the_first_at_midnight_utc() {
        let month = CostPeriod::Month;
        assert_eq!(
            month.start(at(2026, 2, 17, 23, 59, 59)),
            at(2026, 2, 1, 0, 0, 0)
        );
        assert_eq!(
            month.start(at(2026, 3, 1, 0, 0, 0)),
            at(2026, 3, 1, 0, 0, 0)
        );
        assert_eq!(
            month.next_start(at(2026, 12, 15, 0, 0, 0)),
            at(2027, 1, 1, 0, 0, 0)
        );
        assert_eq!(
            month.next_start(at(2028, 2, 29, 10, 0, 0)),
            at(2028, 3, 1, 0, 0, 0)
        );
        assert_eq!(
            month.next_start(at(2026, 1, 31, 10, 0, 0)),
            at(2026, 2, 1, 0, 0, 0)
        );
    }

    #[test]
    fn period_names_round_trip() {
        for period in [CostPeriod::Day, CostPeriod::Month] {
            assert_eq!(CostPeriod::from_str(period.as_str()).ok(), Some(period));
            assert_eq!(period.to_string(), period.as_str());
        }
        assert!(CostPeriod::from_str("week").is_err());
        assert_eq!(to_string(&CostPeriod::Day).unwrap(), "\"day\"");
    }

    #[test]
    fn zero_cap_is_allowed() {
        assert_eq!(
            RoleCostCap::per_day(Decimal::ZERO).limit_usd(),
            Decimal::ZERO
        );
    }

    #[test]
    #[should_panic(expected = "must not be negative")]
    fn negative_cap_panics() {
        let _ = RoleCostCap::per_month(Decimal::NEGATIVE_ONE);
    }

    #[test]
    #[should_panic(expected = "invalid agent role name")]
    fn binding_with_an_invalid_name_panics() {
        let _ = AgentRoleBinding::new("Reviewer", None);
    }

    #[test]
    fn binding_without_cap_counts_over_a_month() {
        let binding = AgentRoleBinding::new("reviewer", None);
        assert_eq!(binding.cost_cap_usd, None);
        assert_eq!(binding.period, CostPeriod::Month);
    }

    #[test]
    fn binding_takes_the_cap_and_period() {
        let binding = AgentRoleBinding::new("reviewer", Some(RoleCostCap::per_day(Decimal::TEN)));
        assert_eq!(binding.cost_cap_usd, Some(Decimal::TEN));
        assert_eq!(binding.period, CostPeriod::Day);
    }
}
