//! [`CronSchedule`] -- validated cron expression newtype.
//!
//! Wraps [`croner::Cron`] to guarantee that any `CronSchedule` value
//! holds a syntactically valid cron expression. Construction
//! is fallible; once built the value is safe to pass to
//! `tokio_cron_scheduler` without further validation.

use std::fmt;
use std::str::FromStr;

use croner::Cron;
use serde::{Deserialize, Serialize};

/// A validated cron expression.
///
/// Internally wraps a [`croner::Cron`], guaranteeing that the expression
/// has been parsed and validated at construction time.
///
/// [`as_str`](CronSchedule::as_str) returns the original expression
/// as provided by the user, not the normalized form.
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
        })
    }

    /// Returns the original cron expression string as provided to [`new`](Self::new).
    pub fn as_str(&self) -> &str {
        &self.raw
    }
}

impl fmt::Display for CronSchedule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw)
    }
}

impl PartialEq for CronSchedule {
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner
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
}
