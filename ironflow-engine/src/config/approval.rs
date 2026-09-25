//! [`ApprovalConfig`] -- configuration for human approval gates.

use std::time::Duration;

use ironflow_store::entities::Assignee;
use serde::{Deserialize, Serialize};

use super::{Approvers, EscalationPolicy};

/// Configuration for a human approval step.
///
/// When the workflow reaches an approval step, the run transitions to
/// `AwaitingApproval` and waits for a human to approve or reject via
/// the API.
///
/// A gate can carry an SLA: [`with_deadline`](Self::with_deadline) arms a timer
/// persisted alongside the step, and [`on_timeout`](Self::on_timeout) says what
/// happens when it fires. The timer lives in the database, so it survives an API
/// or worker restart.
///
/// A gate can also require several approvers:
/// [`requiring`](Self::requiring) takes the [`Approvers`] the handler computed
/// in Rust, from its typed input and earlier step outputs. They decide how many
/// distinct approvals the gate needs and which groups may vote. A config
/// without approvers is resolved by one approval from anyone allowed to answer
/// the gate.
///
/// # Examples
///
/// ```
/// use ironflow_engine::config::{ApprovalConfig, Approvers};
///
/// let config = ApprovalConfig::new("Deploy to production?");
/// assert_eq!(config.message(), "Deploy to production?");
/// assert!(config.timeout_seconds().is_none());
///
/// let payment = ApprovalConfig::new("Release the payment?")
///     .requiring(Approvers::at_least(2).from_groups(["finance"]).because("amount > 10k"));
/// assert_eq!(payment.approvers().map(Approvers::required), Some(2));
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalConfig {
    message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    timeout_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    deadline_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    on_timeout: Option<EscalationPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    assignee: Option<Assignee>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    approvers: Option<Approvers>,
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
            deadline_secs: None,
            on_timeout: None,
            assignee: None,
            approvers: None,
        }
    }

    /// Set an auto-reject timeout in seconds.
    ///
    /// If no approval or rejection is received within this duration,
    /// the run is automatically rejected (marked as Failed).
    ///
    /// This is the legacy spelling of [`with_deadline_secs`](Self::with_deadline_secs)
    /// with an implicit [`EscalationPolicy::AutoReject`]. It is now actually
    /// enforced by the escalator; a config that sets both keeps the explicit
    /// deadline.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::{ApprovalConfig, EscalationPolicy};
    ///
    /// let config = ApprovalConfig::new("Approve?")
    ///     .with_timeout_seconds(3600);
    /// assert_eq!(config.timeout_seconds(), Some(3600));
    /// assert_eq!(config.effective_deadline_secs(), Some(3600));
    /// assert_eq!(config.effective_policy(), EscalationPolicy::AutoReject);
    /// ```
    pub fn with_timeout_seconds(mut self, seconds: u64) -> Self {
        self.timeout_seconds = Some(seconds);
        self
    }

    /// Set the SLA deadline of this gate.
    ///
    /// Sub-second precision is dropped: the deadline is stored in whole seconds.
    ///
    /// # Panics
    ///
    /// Panics if `deadline` rounds down to zero seconds.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    /// use ironflow_engine::config::ApprovalConfig;
    ///
    /// let config = ApprovalConfig::new("Approve?")
    ///     .with_deadline(Duration::from_secs(1800));
    /// assert_eq!(config.deadline(), Some(Duration::from_secs(1800)));
    /// ```
    pub fn with_deadline(self, deadline: Duration) -> Self {
        self.with_deadline_secs(deadline.as_secs())
    }

    /// Set the SLA deadline of this gate, in seconds.
    ///
    /// # Panics
    ///
    /// Panics if `secs` is zero: a gate that expires the instant it opens can
    /// never be approved by a human.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::ApprovalConfig;
    ///
    /// let config = ApprovalConfig::new("Approve?").with_deadline_secs(1800);
    /// assert_eq!(config.deadline_secs(), Some(1800));
    /// ```
    pub fn with_deadline_secs(mut self, secs: u64) -> Self {
        assert!(secs > 0, "approval deadline must be greater than zero");
        self.deadline_secs = Some(secs);
        self
    }

    /// Set the policy applied when the deadline fires.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::{ApprovalConfig, EscalationPolicy};
    ///
    /// let config = ApprovalConfig::new("Approve?")
    ///     .with_deadline_secs(3600)
    ///     .on_timeout(EscalationPolicy::AutoApprove);
    /// assert_eq!(config.effective_policy(), EscalationPolicy::AutoApprove);
    /// ```
    pub fn on_timeout(mut self, policy: EscalationPolicy) -> Self {
        self.on_timeout = Some(policy);
        self
    }

    /// Assign the gate to a user or group.
    ///
    /// # Panics
    ///
    /// Panics if the assignee name is empty or only whitespace.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::ApprovalConfig;
    /// use ironflow_store::entities::Assignee;
    ///
    /// let config = ApprovalConfig::new("Approve?").assigned_to(Assignee::group("release-managers"));
    /// assert_eq!(config.assignee(), Some(&Assignee::group("release-managers")));
    /// ```
    pub fn assigned_to(mut self, assignee: Assignee) -> Self {
        assert!(
            !assignee.name().trim().is_empty(),
            "approval assignee must not be empty"
        );
        self.assignee = Some(assignee);
        self
    }

    /// Require the given approvers. A later call replaces an earlier one.
    ///
    /// The engine records them on the gate when it opens, as an
    /// [`ApprovalRequirement`](crate::config::ApprovalRequirement); that record
    /// stays the source of truth on replay and resume.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::{ApprovalConfig, Approvers};
    ///
    /// let config = ApprovalConfig::new("Approve?")
    ///     .requiring(Approvers::at_least(3).from_groups(["finance", "board"]));
    /// assert_eq!(config.approvers().map(Approvers::required), Some(3));
    /// ```
    pub fn requiring(mut self, approvers: Approvers) -> Self {
        self.approvers = Some(approvers);
        self
    }

    /// The approvers this gate requires, if any were set.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::ApprovalConfig;
    ///
    /// assert!(ApprovalConfig::new("Approve?").approvers().is_none());
    /// ```
    pub fn approvers(&self) -> Option<&Approvers> {
        self.approvers.as_ref()
    }

    /// The approval message displayed to reviewers.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Optional auto-reject timeout in seconds.
    pub fn timeout_seconds(&self) -> Option<u64> {
        self.timeout_seconds
    }

    /// The configured SLA deadline, if any.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    /// use ironflow_engine::config::ApprovalConfig;
    ///
    /// let config = ApprovalConfig::new("Approve?").with_deadline_secs(60);
    /// assert_eq!(config.deadline(), Some(Duration::from_secs(60)));
    /// ```
    pub fn deadline(&self) -> Option<Duration> {
        self.deadline_secs.map(Duration::from_secs)
    }

    /// The configured SLA deadline in seconds, if any.
    pub fn deadline_secs(&self) -> Option<u64> {
        self.deadline_secs
    }

    /// The configured escalation policy, if any.
    pub fn on_timeout_policy(&self) -> Option<&EscalationPolicy> {
        self.on_timeout.as_ref()
    }

    /// The user or group the gate is assigned to, if any.
    pub fn assignee(&self) -> Option<&Assignee> {
        self.assignee.as_ref()
    }

    /// The deadline actually enforced, in seconds.
    ///
    /// [`with_deadline_secs`](Self::with_deadline_secs) wins; the legacy
    /// [`with_timeout_seconds`](Self::with_timeout_seconds) is honoured as a
    /// fallback so configs written before escalation existed finally behave the
    /// way their documentation always promised.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::ApprovalConfig;
    ///
    /// let legacy = ApprovalConfig::new("Approve?").with_timeout_seconds(7200);
    /// assert_eq!(legacy.effective_deadline_secs(), Some(7200));
    ///
    /// let both = legacy.with_deadline_secs(60);
    /// assert_eq!(both.effective_deadline_secs(), Some(60));
    ///
    /// assert_eq!(ApprovalConfig::new("Approve?").effective_deadline_secs(), None);
    /// ```
    pub fn effective_deadline_secs(&self) -> Option<u64> {
        self.deadline_secs.or(self.timeout_seconds)
    }

    /// The policy applied when the deadline fires. Defaults to
    /// [`EscalationPolicy::AutoReject`], matching the documented meaning of
    /// `timeout_seconds`.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::{ApprovalConfig, EscalationPolicy};
    ///
    /// let config = ApprovalConfig::new("Approve?").with_deadline_secs(60);
    /// assert_eq!(config.effective_policy(), EscalationPolicy::AutoReject);
    /// ```
    pub fn effective_policy(&self) -> EscalationPolicy {
        self.on_timeout
            .clone()
            .unwrap_or(EscalationPolicy::AutoReject)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{from_str, from_value, json, to_string};

    use super::*;
    use crate::config::NotificationTarget;

    #[test]
    fn new_sets_message() {
        let config = ApprovalConfig::new("Deploy?");
        assert_eq!(config.message(), "Deploy?");
        assert!(config.timeout_seconds().is_none());
        assert!(config.deadline_secs().is_none());
        assert!(config.on_timeout_policy().is_none());
        assert!(config.assignee().is_none());
    }

    #[test]
    fn with_timeout() {
        let config = ApprovalConfig::new("Approve?").with_timeout_seconds(7200);
        assert_eq!(config.timeout_seconds(), Some(7200));
    }

    #[test]
    fn with_deadline_stores_whole_seconds() {
        let config = ApprovalConfig::new("Approve?").with_deadline(Duration::from_millis(90_500));
        assert_eq!(config.deadline_secs(), Some(90));
        assert_eq!(config.deadline(), Some(Duration::from_secs(90)));
    }

    #[test]
    fn on_timeout_stores_the_policy() {
        let config = ApprovalConfig::new("Approve?").on_timeout(EscalationPolicy::AutoApprove);
        assert_eq!(
            config.on_timeout_policy(),
            Some(&EscalationPolicy::AutoApprove)
        );
    }

    #[test]
    fn assigned_to_stores_the_assignee() {
        let config =
            ApprovalConfig::new("Approve?").assigned_to(Assignee::group("release-managers"));
        assert_eq!(
            config.assignee(),
            Some(&Assignee::group("release-managers"))
        );
    }

    #[test]
    fn effective_deadline_prefers_the_explicit_deadline() {
        let config = ApprovalConfig::new("Approve?")
            .with_timeout_seconds(7200)
            .with_deadline_secs(60);
        assert_eq!(config.effective_deadline_secs(), Some(60));
    }

    #[test]
    fn effective_deadline_falls_back_to_the_legacy_timeout() {
        let config = ApprovalConfig::new("Approve?").with_timeout_seconds(7200);
        assert_eq!(config.effective_deadline_secs(), Some(7200));
    }

    #[test]
    fn effective_deadline_is_none_without_any_timer() {
        assert_eq!(
            ApprovalConfig::new("Approve?").effective_deadline_secs(),
            None
        );
    }

    #[test]
    fn effective_policy_defaults_to_auto_reject() {
        let config = ApprovalConfig::new("Approve?").with_deadline_secs(60);
        assert_eq!(config.effective_policy(), EscalationPolicy::AutoReject);
    }

    #[test]
    fn effective_policy_returns_the_configured_policy() {
        let policy = EscalationPolicy::Chain(vec![
            EscalationPolicy::Notify(vec![NotificationTarget::Webhook {
                url: "https://example.com/sla".to_string(),
            }]),
            EscalationPolicy::AutoReject,
        ]);
        let config = ApprovalConfig::new("Approve?")
            .with_deadline_secs(60)
            .on_timeout(policy.clone());
        assert_eq!(config.effective_policy(), policy);
    }

    #[test]
    #[should_panic(expected = "approval deadline must be greater than zero")]
    fn with_deadline_secs_rejects_zero() {
        let _ = ApprovalConfig::new("Approve?").with_deadline_secs(0);
    }

    #[test]
    #[should_panic(expected = "approval assignee must not be empty")]
    fn assigned_to_rejects_blank() {
        let _ = ApprovalConfig::new("Approve?").assigned_to(Assignee::group("  "));
    }

    #[test]
    fn serde_roundtrip() {
        let config = ApprovalConfig::new("Deploy to prod?")
            .with_timeout_seconds(3600)
            .with_deadline_secs(1800)
            .on_timeout(EscalationPolicy::Escalate(Assignee::group("sre-oncall")))
            .assigned_to(Assignee::group("release-managers"));

        let json = serde_json::to_string(&config).expect("serialize");
        let back: ApprovalConfig = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(back.message(), config.message());
        assert_eq!(back.timeout_seconds(), config.timeout_seconds());
        assert_eq!(back.deadline_secs(), config.deadline_secs());
        assert_eq!(back.on_timeout_policy(), config.on_timeout_policy());
        assert_eq!(back.assignee(), config.assignee());
    }

    #[test]
    fn serde_minimal() {
        let config = ApprovalConfig::new("Approve?");
        let json = serde_json::to_string(&config).expect("serialize");
        assert!(!json.contains("timeout_seconds"));
        assert!(!json.contains("deadline_secs"));
        assert!(!json.contains("on_timeout"));
        assert!(!json.contains("assignee"));
        assert!(!json.contains("approvers"));
    }

    #[test]
    fn requiring_stores_the_approvers() {
        let approvers = Approvers::at_least(2)
            .from_groups(["finance"])
            .because("amount > 10k");
        let config = ApprovalConfig::new("Release the payment?").requiring(approvers.clone());
        assert_eq!(config.approvers(), Some(&approvers));
    }

    #[test]
    fn requiring_twice_keeps_the_last_approvers() {
        let config = ApprovalConfig::new("Approve?")
            .requiring(Approvers::at_least(3))
            .requiring(Approvers::any());
        assert_eq!(config.approvers(), Some(&Approvers::any()));
    }

    #[test]
    fn serde_roundtrip_with_approvers() {
        let config = ApprovalConfig::new("Release the payment?").requiring(
            Approvers::at_least(3)
                .from_groups(["finance", "board"])
                .because("amount > 100k"),
        );
        let json = to_string(&config).expect("serialize");
        let back: ApprovalConfig = from_str(&json).expect("deserialize");

        assert_eq!(back.approvers(), config.approvers());
        assert_eq!(to_string(&back).expect("serialize"), json);
    }

    #[test]
    fn serde_accepts_a_config_written_before_approvers_existed() {
        let raw = r#"{"message":"Approve?","assignee":"user:alice"}"#;
        let config: ApprovalConfig = from_str(raw).expect("deserialize");

        assert!(config.approvers().is_none());
        assert_eq!(config.assignee(), Some(&Assignee::user("alice")));
    }

    #[test]
    fn serde_ignores_the_rules_of_a_config_written_by_approval_rules() {
        // Step inputs recorded before approval rules were removed still load.
        let config: ApprovalConfig = from_value(json!({
            "message": "Approve?",
            "rules": [{"condition": "payload.amount > 10000", "required_approvers": 2}],
        }))
        .expect("deserialize");

        assert_eq!(config.message(), "Approve?");
        assert!(config.approvers().is_none());
    }

    #[test]
    fn serde_rejects_zero_approvers() {
        let result = from_value::<ApprovalConfig>(json!({
            "message": "Approve?",
            "approvers": {"required_approvers": 0},
        }));
        assert!(result.is_err());
    }

    #[test]
    fn serde_accepts_a_config_written_before_escalation_existed() {
        let config: ApprovalConfig =
            serde_json::from_str(r#"{"message":"Approve?","timeout_seconds":60}"#)
                .expect("deserialize");

        assert_eq!(config.effective_deadline_secs(), Some(60));
        assert_eq!(config.effective_policy(), EscalationPolicy::AutoReject);
    }
}
