//! Configuration for delay (timed pause) steps.
//!
//! A [`DelayConfig`] pauses a workflow run for a specified duration.
//! The worker releases its slot while the run sleeps and resumes
//! it automatically when the delay elapses.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Configuration for a delay step.
///
/// The delay is stored as whole seconds for serialization simplicity.
/// Sub-second precision is not needed for workflow pauses (cooldowns,
/// rate limiting, scheduled retries).
///
/// # Examples
///
/// ```
/// use ironflow_engine::config::delay::DelayConfig;
///
/// let config = DelayConfig::from_secs(300);
/// assert_eq!(config.duration(), std::time::Duration::from_secs(300));
///
/// let zero = DelayConfig::from_secs(0);
/// assert!(zero.is_zero());
/// ```
///
/// # Panics
///
/// [`DelayConfig::from_secs`] does not panic. Invalid durations
/// (negative or overflow) cannot be constructed because the inner
/// value is `u64`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelayConfig {
    /// Delay duration in seconds.
    duration_secs: u64,
}

impl DelayConfig {
    /// Create a delay configuration with the given number of seconds.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::delay::DelayConfig;
    ///
    /// let config = DelayConfig::from_secs(60);
    /// assert_eq!(config.duration_secs(), 60);
    /// ```
    pub fn from_secs(secs: u64) -> Self {
        Self {
            duration_secs: secs,
        }
    }

    /// The delay duration as a [`Duration`].
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::delay::DelayConfig;
    /// use std::time::Duration;
    ///
    /// assert_eq!(DelayConfig::from_secs(10).duration(), Duration::from_secs(10));
    /// ```
    pub fn duration(&self) -> Duration {
        Duration::from_secs(self.duration_secs)
    }

    /// The raw seconds value.
    pub fn duration_secs(&self) -> u64 {
        self.duration_secs
    }

    /// Whether the delay is zero (immediate completion, no sleeping).
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::delay::DelayConfig;
    ///
    /// assert!(DelayConfig::from_secs(0).is_zero());
    /// assert!(!DelayConfig::from_secs(1).is_zero());
    /// ```
    pub fn is_zero(&self) -> bool {
        self.duration_secs == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_config_from_secs() {
        let config = DelayConfig::from_secs(300);
        assert_eq!(config.duration_secs(), 300);
        assert_eq!(config.duration(), Duration::from_secs(300));
        assert!(!config.is_zero());
    }

    #[test]
    fn delay_config_zero() {
        let config = DelayConfig::from_secs(0);
        assert!(config.is_zero());
        assert_eq!(config.duration(), Duration::ZERO);
    }

    #[test]
    fn delay_config_serde_roundtrip() {
        let config = DelayConfig::from_secs(60);
        let json = serde_json::to_string(&config).expect("serialize");
        let back: DelayConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(config.duration_secs(), back.duration_secs());
    }
}
