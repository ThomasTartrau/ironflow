//! [`HumanInputConfig`] -- configuration for typed human input steps.

use std::time::Duration;

use ironflow_store::entities::Assignee;
use serde::{Deserialize, Serialize};

use super::{ApprovalConfig, Approvers, EscalationPolicy};

/// Key of the JSON schema of the expected answer in the stored step input.
///
/// A human input step stores its flattened [`HumanInputConfig`] in
/// `step.input`, plus the JSON schema of the answer type under this key. The
/// API validates a submitted answer against it before the run resumes.
///
/// # Examples
///
/// ```
/// use ironflow_engine::config::HUMAN_INPUT_SCHEMA_KEY;
///
/// assert_eq!(HUMAN_INPUT_SCHEMA_KEY, "schema");
/// ```
pub const HUMAN_INPUT_SCHEMA_KEY: &str = "schema";

/// Configuration for a typed human input step.
///
/// When the workflow reaches a human input step, the run transitions to
/// `AwaitingApproval` and waits for a human to submit an answer matching the
/// JSON schema of the expected type, or to reject the request, via the API.
///
/// The step reuses the approval gate machinery: a deadline, an escalation
/// policy, an assignee and the [`Approvers`] allowed to answer. The first valid
/// answer wins; [`Approvers::at_least`] only restricts who may answer.
///
/// A human input cannot be auto-approved on timeout since there is no value to
/// fill in: [`on_timeout`](Self::on_timeout) refuses
/// [`EscalationPolicy::AutoApprove`].
///
/// # Examples
///
/// ```
/// use std::time::Duration;
/// use ironflow_engine::config::{EscalationPolicy, HumanInputConfig};
///
/// let config = HumanInputConfig::new("Answer the clarification questions")
///     .with_deadline(Duration::from_secs(3600))
///     .on_timeout(EscalationPolicy::AutoReject);
/// assert_eq!(config.message(), "Answer the clarification questions");
/// assert_eq!(config.deadline_secs(), Some(3600));
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HumanInputConfig {
    #[serde(flatten)]
    gate: ApprovalConfig,
}

impl HumanInputConfig {
    /// Create a new human input config with the given message.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::HumanInputConfig;
    ///
    /// let config = HumanInputConfig::new("Which environment?");
    /// assert_eq!(config.message(), "Which environment?");
    /// ```
    pub fn new(message: &str) -> Self {
        Self {
            gate: ApprovalConfig::new(message),
        }
    }

    /// Set the SLA deadline of this input.
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
    /// use ironflow_engine::config::HumanInputConfig;
    ///
    /// let config = HumanInputConfig::new("Answer?").with_deadline(Duration::from_secs(1800));
    /// assert_eq!(config.deadline(), Some(Duration::from_secs(1800)));
    /// ```
    pub fn with_deadline(self, deadline: Duration) -> Self {
        Self {
            gate: self.gate.with_deadline(deadline),
        }
    }

    /// Set the SLA deadline of this input, in seconds.
    ///
    /// # Panics
    ///
    /// Panics if `secs` is zero.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::HumanInputConfig;
    ///
    /// let config = HumanInputConfig::new("Answer?").with_deadline_secs(1800);
    /// assert_eq!(config.deadline_secs(), Some(1800));
    /// ```
    pub fn with_deadline_secs(self, secs: u64) -> Self {
        Self {
            gate: self.gate.with_deadline_secs(secs),
        }
    }

    /// Set the policy applied when the deadline fires.
    ///
    /// # Panics
    ///
    /// Panics if the policy is [`EscalationPolicy::AutoApprove`] or a
    /// [`EscalationPolicy::Chain`] containing it: a human input has no value
    /// to fill in automatically.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::{EscalationPolicy, HumanInputConfig};
    ///
    /// let config = HumanInputConfig::new("Answer?")
    ///     .with_deadline_secs(3600)
    ///     .on_timeout(EscalationPolicy::AutoReject);
    /// assert_eq!(config.effective_policy(), EscalationPolicy::AutoReject);
    /// ```
    ///
    /// ```should_panic
    /// use ironflow_engine::config::{EscalationPolicy, HumanInputConfig};
    ///
    /// let _ = HumanInputConfig::new("Answer?").on_timeout(EscalationPolicy::AutoApprove);
    /// ```
    pub fn on_timeout(self, policy: EscalationPolicy) -> Self {
        assert!(
            !allows_auto_approve(&policy),
            "a human input step cannot be auto-approved on timeout"
        );
        Self {
            gate: self.gate.on_timeout(policy),
        }
    }

    /// Assign the input to a user or group.
    ///
    /// # Panics
    ///
    /// Panics if the assignee name is empty or only whitespace.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::HumanInputConfig;
    /// use ironflow_store::entities::Assignee;
    ///
    /// let config = HumanInputConfig::new("Answer?").assigned_to(Assignee::user("alice"));
    /// assert_eq!(config.assignee(), Some(&Assignee::user("alice")));
    /// ```
    pub fn assigned_to(self, assignee: Assignee) -> Self {
        Self {
            gate: self.gate.assigned_to(assignee),
        }
    }

    /// Restrict who may answer. A later call replaces an earlier one.
    ///
    /// The first valid answer resolves the step, whatever the required count.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::{Approvers, HumanInputConfig};
    ///
    /// let config = HumanInputConfig::new("Answer?")
    ///     .requiring(Approvers::any().from_groups(["product"]));
    /// assert!(config.approvers().is_some());
    /// ```
    pub fn requiring(self, approvers: Approvers) -> Self {
        Self {
            gate: self.gate.requiring(approvers),
        }
    }

    /// The message displayed to the person answering.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::HumanInputConfig;
    ///
    /// assert_eq!(HumanInputConfig::new("Answer?").message(), "Answer?");
    /// ```
    pub fn message(&self) -> &str {
        self.gate.message()
    }

    /// The configured SLA deadline, if any.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    /// use ironflow_engine::config::HumanInputConfig;
    ///
    /// let config = HumanInputConfig::new("Answer?").with_deadline_secs(60);
    /// assert_eq!(config.deadline(), Some(Duration::from_secs(60)));
    /// ```
    pub fn deadline(&self) -> Option<Duration> {
        self.gate.deadline()
    }

    /// The configured SLA deadline in seconds, if any.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::HumanInputConfig;
    ///
    /// assert!(HumanInputConfig::new("Answer?").deadline_secs().is_none());
    /// ```
    pub fn deadline_secs(&self) -> Option<u64> {
        self.gate.deadline_secs()
    }

    /// The configured escalation policy, if any.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::HumanInputConfig;
    ///
    /// assert!(HumanInputConfig::new("Answer?").on_timeout_policy().is_none());
    /// ```
    pub fn on_timeout_policy(&self) -> Option<&EscalationPolicy> {
        self.gate.on_timeout_policy()
    }

    /// The user or group the input is assigned to, if any.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::HumanInputConfig;
    ///
    /// assert!(HumanInputConfig::new("Answer?").assignee().is_none());
    /// ```
    pub fn assignee(&self) -> Option<&Assignee> {
        self.gate.assignee()
    }

    /// The approvers allowed to answer, if any were set.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::HumanInputConfig;
    ///
    /// assert!(HumanInputConfig::new("Answer?").approvers().is_none());
    /// ```
    pub fn approvers(&self) -> Option<&Approvers> {
        self.gate.approvers()
    }

    /// The deadline actually enforced, in seconds.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::HumanInputConfig;
    ///
    /// let config = HumanInputConfig::new("Answer?").with_deadline_secs(60);
    /// assert_eq!(config.effective_deadline_secs(), Some(60));
    /// ```
    pub fn effective_deadline_secs(&self) -> Option<u64> {
        self.gate.effective_deadline_secs()
    }

    /// The policy applied when the deadline fires. Defaults to
    /// [`EscalationPolicy::AutoReject`].
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::{EscalationPolicy, HumanInputConfig};
    ///
    /// let config = HumanInputConfig::new("Answer?").with_deadline_secs(60);
    /// assert_eq!(config.effective_policy(), EscalationPolicy::AutoReject);
    /// ```
    pub fn effective_policy(&self) -> EscalationPolicy {
        self.gate.effective_policy()
    }
}

/// Whether `policy` can end up auto-approving the step.
fn allows_auto_approve(policy: &EscalationPolicy) -> bool {
    match policy {
        EscalationPolicy::AutoApprove => true,
        EscalationPolicy::Chain(policies) => policies.iter().any(allows_auto_approve),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{from_str, from_value, json, to_string, to_value};

    use super::*;
    use crate::config::NotificationTarget;

    fn webhook() -> EscalationPolicy {
        EscalationPolicy::Notify(vec![NotificationTarget::Webhook {
            url: "https://example.com/sla".to_string(),
        }])
    }

    #[test]
    fn new_sets_message() {
        let config = HumanInputConfig::new("Answer?");
        assert_eq!(config.message(), "Answer?");
        assert!(config.deadline_secs().is_none());
        assert!(config.on_timeout_policy().is_none());
        assert!(config.assignee().is_none());
        assert!(config.approvers().is_none());
    }

    #[test]
    fn builder_values_are_stored() {
        let config = HumanInputConfig::new("Answer?")
            .with_deadline(Duration::from_secs(120))
            .on_timeout(EscalationPolicy::AutoReject)
            .assigned_to(Assignee::group("product"))
            .requiring(Approvers::at_least(2).from_groups(["product"]));

        assert_eq!(config.deadline(), Some(Duration::from_secs(120)));
        assert_eq!(config.effective_deadline_secs(), Some(120));
        assert_eq!(
            config.on_timeout_policy(),
            Some(&EscalationPolicy::AutoReject)
        );
        assert_eq!(config.assignee(), Some(&Assignee::group("product")));
        assert_eq!(config.approvers().map(Approvers::required), Some(2));
    }

    #[test]
    fn with_deadline_secs_is_stored() {
        let config = HumanInputConfig::new("Answer?").with_deadline_secs(30);
        assert_eq!(config.deadline_secs(), Some(30));
    }

    #[test]
    #[should_panic(expected = "approval deadline must be greater than zero")]
    fn with_deadline_secs_rejects_zero() {
        let _ = HumanInputConfig::new("Answer?").with_deadline_secs(0);
    }

    #[test]
    fn on_timeout_accepts_policies_without_auto_approve() {
        let reject = HumanInputConfig::new("Answer?").on_timeout(EscalationPolicy::AutoReject);
        assert_eq!(reject.effective_policy(), EscalationPolicy::AutoReject);

        let notify = HumanInputConfig::new("Answer?").on_timeout(webhook());
        assert_eq!(notify.effective_policy(), webhook());

        let chain = EscalationPolicy::Chain(vec![webhook(), EscalationPolicy::AutoReject]);
        let chained = HumanInputConfig::new("Answer?").on_timeout(chain.clone());
        assert_eq!(chained.effective_policy(), chain);
    }

    #[test]
    #[should_panic(expected = "a human input step cannot be auto-approved on timeout")]
    fn on_timeout_rejects_auto_approve() {
        let _ = HumanInputConfig::new("Answer?").on_timeout(EscalationPolicy::AutoApprove);
    }

    #[test]
    #[should_panic(expected = "a human input step cannot be auto-approved on timeout")]
    fn on_timeout_rejects_a_chain_with_auto_approve() {
        let _ = HumanInputConfig::new("Answer?").on_timeout(EscalationPolicy::Chain(vec![
            webhook(),
            EscalationPolicy::AutoApprove,
        ]));
    }

    #[test]
    fn serde_roundtrip() {
        let config = HumanInputConfig::new("Answer?")
            .with_deadline_secs(600)
            .assigned_to(Assignee::user("alice"))
            .requiring(Approvers::any().from_groups(["product"]));

        let json = to_string(&config).expect("serialize");
        let back: HumanInputConfig = from_str(&json).expect("deserialize");

        assert_eq!(back.message(), config.message());
        assert_eq!(back.deadline_secs(), config.deadline_secs());
        assert_eq!(back.assignee(), config.assignee());
        assert_eq!(back.approvers(), config.approvers());
        assert_eq!(to_string(&back).expect("serialize"), json);
    }

    #[test]
    fn serialized_config_reads_as_an_approval_config() {
        let config = HumanInputConfig::new("Answer?").with_deadline_secs(600);
        let mut value = to_value(&config).expect("serialize");
        value.as_object_mut().expect("object").insert(
            HUMAN_INPUT_SCHEMA_KEY.to_string(),
            json!({"type": "object"}),
        );

        let approval: ApprovalConfig = from_value(value).expect("deserialize");
        assert_eq!(approval.message(), "Answer?");
        assert_eq!(approval.deadline_secs(), Some(600));
    }
}
