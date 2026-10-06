//! [`CronSchedule`] -- validated cron expression newtype.
//!
//! Wraps [`croner::Cron`] to guarantee that any `CronSchedule` value
//! holds a syntactically valid cron expression. Construction
//! is fallible; once built the value is safe to pass to
//! `tokio_cron_scheduler` without further validation.
//!
//! A schedule also carries a [`SchedulePolicy`]: what to do with missed
//! occurrences, with overlapping runs, and the timezone the expression is
//! evaluated in.

use std::fmt;
use std::str::FromStr;
use std::time::Duration;

use chrono_tz::Tz;
use croner::Cron;
use ironflow_store::entities::{
    MAX_CATCHUP_MAX, MAX_CATCHUP_WINDOW_SECS, MIN_CATCHUP_MAX, MIN_CATCHUP_WINDOW_SECS,
};
use serde::{Deserialize, Serialize};

pub use ironflow_store::entities::{CatchupPolicy, OverlapPolicy, SchedulePolicy};

/// A validated cron expression.
///
/// Internally wraps a [`croner::Cron`], guaranteeing that the expression
/// has been parsed and validated at construction time.
///
/// [`as_str`](CronSchedule::as_str) returns the original expression
/// as provided by the user, not the normalized form.
///
/// The schedule also carries a [`SchedulePolicy`], set with the `with_*`
/// builder methods. The policy is not part of the serialized form: a
/// `CronSchedule` serializes to its raw expression, and deserializing one
/// gives the default policy.
///
/// # Examples
///
/// ```
/// use ironflow_engine::schedule::CronSchedule;
///
/// let sched = CronSchedule::new("0 0 * * * *").unwrap();
/// assert_eq!(sched.as_str(), "0 0 * * * *");
///
/// let bad = CronSchedule::new("not a cron");
/// assert!(bad.is_err());
/// ```
#[derive(Debug, Clone)]
pub struct CronSchedule {
    inner: Cron,
    raw: String,
    policy: SchedulePolicy,
}

impl CronSchedule {
    /// Parse and validate a cron expression.
    ///
    /// Accepts 5-field (standard) or 6-field (with seconds) expressions,
    /// as supported by [`croner`].
    ///
    /// # Errors
    ///
    /// Returns an error string if the expression is syntactically invalid.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::schedule::CronSchedule;
    ///
    /// assert!(CronSchedule::new("0 */5 * * * *").is_ok());
    /// assert!(CronSchedule::new("garbage").is_err());
    /// ```
    pub fn new(expression: &str) -> Result<Self, String> {
        let inner = Cron::from_str(expression)
            .map_err(|e| format!("invalid cron expression '{expression}': {e}"))?;
        Ok(Self {
            inner,
            raw: expression.to_string(),
            policy: SchedulePolicy::default(),
        })
    }

    /// Returns the original cron expression string as provided to [`new`](Self::new).
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    /// Evaluate the expression in an IANA timezone instead of UTC.
    ///
    /// Occurrences follow the wall clock of the timezone across daylight
    /// saving changes: `0 9 * * *` in `Europe/Paris` fires at 9:00 Paris
    /// time in winter and in summer. An occurrence in an hour skipped in
    /// spring fires once at the end of the gap; an occurrence in an hour
    /// repeated in autumn fires once, on its first pass.
    ///
    /// # Errors
    ///
    /// Returns an error string if `tz` is not a known IANA timezone name.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::schedule::CronSchedule;
    ///
    /// let sched = CronSchedule::new("0 9 * * *")?.with_timezone("Europe/Paris")?;
    /// assert_eq!(sched.policy().timezone.name(), "Europe/Paris");
    ///
    /// assert!(CronSchedule::new("0 9 * * *")?.with_timezone("Mars/Olympus").is_err());
    /// # Ok::<(), String>(())
    /// ```
    pub fn with_timezone(mut self, tz: &str) -> Result<Self, String> {
        let parsed: Tz = tz
            .parse()
            .map_err(|e| format!("invalid timezone '{tz}': {e}"))?;
        self.policy.timezone = parsed;
        Ok(self)
    }

    /// Set what the schedule does with the occurrences it missed while no
    /// server fired it. Defaults to [`CatchupPolicy::Latest`].
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::schedule::{CronSchedule, CatchupPolicy};
    ///
    /// let sched = CronSchedule::new("0 * * * *")?.with_catchup(CatchupPolicy::All);
    /// assert_eq!(sched.policy().catchup, CatchupPolicy::All);
    /// # Ok::<(), String>(())
    /// ```
    pub fn with_catchup(mut self, catchup: CatchupPolicy) -> Self {
        self.policy.catchup = catchup;
        self
    }

    /// Set the most runs created to catch up under [`CatchupPolicy::All`].
    /// The most recent missed occurrences are kept. Defaults to `10`.
    ///
    /// # Panics
    ///
    /// Panics if `max` is not between `1` and `1000`.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::schedule::CronSchedule;
    ///
    /// let sched = CronSchedule::new("0 * * * *")?.with_catchup_max(24);
    /// assert_eq!(sched.policy().catchup_max, 24);
    /// # Ok::<(), String>(())
    /// ```
    pub fn with_catchup_max(mut self, max: u32) -> Self {
        assert!(
            (MIN_CATCHUP_MAX..=MAX_CATCHUP_MAX).contains(&max),
            "catchup_max must be between {MIN_CATCHUP_MAX} and {MAX_CATCHUP_MAX}, got {max}"
        );
        self.policy.catchup_max = max;
        self
    }

    /// Set how far back a missed occurrence is still caught up. Older ones
    /// are dropped. Defaults to one day.
    ///
    /// # Panics
    ///
    /// Panics if `window` is shorter than one minute or longer than 30 days.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    ///
    /// use ironflow_engine::schedule::CronSchedule;
    ///
    /// let sched = CronSchedule::new("0 * * * *")?.with_catchup_window(Duration::from_secs(6 * 3600));
    /// assert_eq!(sched.policy().catchup_window_secs, 21_600);
    /// # Ok::<(), String>(())
    /// ```
    pub fn with_catchup_window(mut self, window: Duration) -> Self {
        let secs = window.as_secs();
        assert!(
            (u64::from(MIN_CATCHUP_WINDOW_SECS)..=u64::from(MAX_CATCHUP_WINDOW_SECS))
                .contains(&secs),
            "catchup window must be between {MIN_CATCHUP_WINDOW_SECS} and {MAX_CATCHUP_WINDOW_SECS} seconds, got {secs}"
        );
        self.policy.catchup_window_secs =
            u32::try_from(secs).expect("catchup window bounded by the assert above");
        self
    }

    /// Set what the schedule does when an occurrence comes while one of its
    /// runs is still active. Defaults to [`OverlapPolicy::Allow`].
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::schedule::{CronSchedule, OverlapPolicy};
    ///
    /// let sched = CronSchedule::new("*/5 * * * *")?.with_overlap(OverlapPolicy::Skip);
    /// assert_eq!(sched.policy().overlap, OverlapPolicy::Skip);
    /// # Ok::<(), String>(())
    /// ```
    pub fn with_overlap(mut self, overlap: OverlapPolicy) -> Self {
        self.policy.overlap = overlap;
        self
    }

    /// Returns the catch-up, overlap and timezone policy of the schedule.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::schedule::{CronSchedule, SchedulePolicy};
    ///
    /// let sched = CronSchedule::new("0 * * * *")?;
    /// assert_eq!(sched.policy(), &SchedulePolicy::default());
    /// # Ok::<(), String>(())
    /// ```
    pub fn policy(&self) -> &SchedulePolicy {
        &self.policy
    }
}

impl fmt::Display for CronSchedule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw)
    }
}

impl PartialEq for CronSchedule {
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner && self.policy == other.policy
    }
}

impl Eq for CronSchedule {}

impl Serialize for CronSchedule {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.raw)
    }
}

impl<'de> Deserialize<'de> for CronSchedule {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::new(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_six_field_expression() {
        let sched = CronSchedule::new("0 0 * * * *").unwrap();
        assert_eq!(sched.as_str(), "0 0 * * * *");
    }

    #[test]
    fn valid_five_field_expression() {
        let sched = CronSchedule::new("*/5 * * * *").unwrap();
        assert_eq!(sched.as_str(), "*/5 * * * *");
    }

    #[test]
    fn valid_complex_expression() {
        let sched = CronSchedule::new("0 30 9 * * MON-FRI").unwrap();
        assert_eq!(sched.as_str(), "0 30 9 * * MON-FRI");
    }

    #[test]
    fn invalid_expression_returns_error() {
        let result = CronSchedule::new("not a cron");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("invalid cron expression"));
    }

    #[test]
    fn empty_expression_returns_error() {
        assert!(CronSchedule::new("").is_err());
    }

    #[test]
    fn display_shows_original_expression() {
        let sched = CronSchedule::new("0 0 12 * * *").unwrap();
        assert_eq!(format!("{sched}"), "0 0 12 * * *");
    }

    #[test]
    fn semantic_equality() {
        let a = CronSchedule::new("0 0 * * * MON-FRI").unwrap();
        let b = CronSchedule::new("0 0 * * * 1-5").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn inequality_on_different_expressions() {
        let a = CronSchedule::new("0 0 * * * *").unwrap();
        let b = CronSchedule::new("0 30 * * * *").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn serde_roundtrip() {
        let sched = CronSchedule::new("0 */5 * * * *").unwrap();
        let json = serde_json::to_string(&sched).unwrap();
        assert_eq!(json, "\"0 */5 * * * *\"");
        let back: CronSchedule = serde_json::from_str(&json).unwrap();
        assert_eq!(back, sched);
    }

    #[test]
    fn deserialize_invalid_expression_fails() {
        let result: Result<CronSchedule, _> = serde_json::from_str("\"garbage\"");
        assert!(result.is_err());
    }

    #[test]
    fn default_policy_is_latest_allow_utc() {
        let sched = CronSchedule::new("0 * * * *").unwrap();
        assert_eq!(sched.policy().catchup, CatchupPolicy::Latest);
        assert_eq!(sched.policy().overlap, OverlapPolicy::Allow);
        assert_eq!(sched.policy().timezone.name(), "UTC");
        assert_eq!(sched.policy(), &SchedulePolicy::default());
    }

    #[test]
    fn with_timezone_accepts_iana_name() {
        let sched = CronSchedule::new("0 9 * * *")
            .unwrap()
            .with_timezone("Europe/Paris")
            .unwrap();
        assert_eq!(sched.policy().timezone.name(), "Europe/Paris");
        assert_eq!(sched.as_str(), "0 9 * * *");
    }

    #[test]
    fn with_timezone_rejects_unknown_name() {
        let err = CronSchedule::new("0 9 * * *")
            .unwrap()
            .with_timezone("Mars/Olympus")
            .unwrap_err();
        assert!(err.contains("invalid timezone 'Mars/Olympus'"), "{err}");
    }

    #[test]
    fn builders_set_the_policy() {
        let sched = CronSchedule::new("0 * * * *")
            .unwrap()
            .with_catchup(CatchupPolicy::All)
            .with_catchup_max(1000)
            .with_catchup_window(Duration::from_secs(60))
            .with_overlap(OverlapPolicy::Skip);
        let policy = sched.policy();
        assert_eq!(policy.catchup, CatchupPolicy::All);
        assert_eq!(policy.catchup_max, 1000);
        assert_eq!(policy.catchup_window_secs, 60);
        assert_eq!(policy.overlap, OverlapPolicy::Skip);
    }

    #[test]
    #[should_panic(expected = "catchup_max must be between 1 and 1000")]
    fn with_catchup_max_zero_panics() {
        let _ = CronSchedule::new("0 * * * *").unwrap().with_catchup_max(0);
    }

    #[test]
    #[should_panic(expected = "catchup window must be between 60 and 2592000 seconds")]
    fn with_catchup_window_below_a_minute_panics() {
        let _ = CronSchedule::new("0 * * * *")
            .unwrap()
            .with_catchup_window(Duration::from_secs(59));
    }

    #[test]
    #[should_panic(expected = "catchup window must be between 60 and 2592000 seconds")]
    fn with_catchup_window_above_thirty_days_panics() {
        let _ = CronSchedule::new("0 * * * *")
            .unwrap()
            .with_catchup_window(Duration::from_secs(2_592_001));
    }

    #[test]
    fn policies_take_part_in_equality() {
        let a = CronSchedule::new("0 * * * *").unwrap();
        let b = CronSchedule::new("0 * * * *")
            .unwrap()
            .with_overlap(OverlapPolicy::Skip);
        let c = CronSchedule::new("0 * * * *")
            .unwrap()
            .with_timezone("Europe/Paris")
            .unwrap();
        assert_ne!(a, b);
        assert_ne!(a, c);
        assert_eq!(b, b.clone());
    }

    #[test]
    fn serialized_form_drops_the_policy() {
        let sched = CronSchedule::new("0 9 * * *")
            .unwrap()
            .with_catchup(CatchupPolicy::Skip);
        let json = serde_json::to_string(&sched).unwrap();
        assert_eq!(json, "\"0 9 * * *\"");
        let back: CronSchedule = serde_json::from_str(&json).unwrap();
        assert_eq!(back.policy(), &SchedulePolicy::default());
    }
}
