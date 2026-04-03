//! [`ApprovalConfig`] -- configuration for human approval gates.

use serde::{Deserialize, Serialize};

/// Configuration for a human approval step.
///
/// When the workflow reaches an approval step, the run transitions to
/// `AwaitingApproval` and waits for a human to approve or reject via
/// the API.
///
/// # Examples
///
/// ```
/// use ironflow_engine::config::ApprovalConfig;
///
/// let config = ApprovalConfig::new("Deploy to production?");
/// assert_eq!(config.message(), "Deploy to production?");
/// assert!(config.timeout_seconds().is_none());
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalConfig {
    message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    timeout_seconds: Option<u64>,
}

impl ApprovalConfig {
    /// Create a new approval config with the given message.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::ApprovalConfig;
    ///
    /// let config = ApprovalConfig::new("Approve this deployment?");
    /// assert_eq!(config.message(), "Approve this deployment?");
    /// ```
    pub fn new(message: &str) -> Self {
        Self {
            message: message.to_string(),
            timeout_seconds: None,
        }
    }

    /// Set an auto-reject timeout in seconds.
    ///
    /// If no approval or rejection is received within this duration,
    /// the run is automatically rejected (marked as Failed).
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::ApprovalConfig;
    ///
    /// let config = ApprovalConfig::new("Approve?")
    ///     .with_timeout_seconds(3600);
    /// assert_eq!(config.timeout_seconds(), Some(3600));
    /// ```
    pub fn with_timeout_seconds(mut self, seconds: u64) -> Self {
        self.timeout_seconds = Some(seconds);
        self
    }

    /// The approval message displayed to reviewers.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Optional auto-reject timeout in seconds.
    pub fn timeout_seconds(&self) -> Option<u64> {
        self.timeout_seconds
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_sets_message() {
        let config = ApprovalConfig::new("Deploy?");
        assert_eq!(config.message(), "Deploy?");
        assert!(config.timeout_seconds().is_none());
    }

    #[test]
    fn with_timeout() {
        let config = ApprovalConfig::new("Approve?").with_timeout_seconds(7200);
        assert_eq!(config.timeout_seconds(), Some(7200));
    }

    #[test]
    fn serde_roundtrip() {
        let config = ApprovalConfig::new("Deploy to prod?").with_timeout_seconds(3600);

        let json = serde_json::to_string(&config).expect("serialize");
        let back: ApprovalConfig = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(back.message(), config.message());
        assert_eq!(back.timeout_seconds(), config.timeout_seconds());
    }

    #[test]
    fn serde_minimal() {
        let config = ApprovalConfig::new("Approve?");
        let json = serde_json::to_string(&config).expect("serialize");
        assert!(!json.contains("timeout_seconds"));
    }
}
