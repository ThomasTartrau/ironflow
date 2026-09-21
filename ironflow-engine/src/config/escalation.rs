//! [`EscalationPolicy`] — what happens when an approval gate misses its deadline.
//!
//! An approval gate configured with
//! [`ApprovalConfig::with_deadline`](super::ApprovalConfig::with_deadline) carries
//! an SLA. When the deadline fires, the API server's escalator applies the policy
//! attached with [`ApprovalConfig::on_timeout`](super::ApprovalConfig::on_timeout).
//!
//! Terminal policies ([`AutoApprove`](EscalationPolicy::AutoApprove),
//! [`AutoReject`](EscalationPolicy::AutoReject)) resolve the gate. Repeating
//! policies ([`Notify`](EscalationPolicy::Notify),
//! [`Escalate`](EscalationPolicy::Escalate)) leave the gate open and restart the
//! timer, so a bare one keeps firing until a human resolves the gate — every
//! cycle leaves an audit entry, so the loop is never silent. Wrap them in a
//! [`Chain`](EscalationPolicy::Chain) to advance one policy per expiry instead.

use serde::{Deserialize, Serialize};

/// What to do when an approval gate misses its SLA deadline.
///
/// # Examples
///
/// ```
/// use ironflow_engine::config::{EscalationPolicy, NotificationTarget};
///
/// // Warn the on-call channel after one hour, give up after two.
/// let policy = EscalationPolicy::Chain(vec![
///     EscalationPolicy::Notify(vec![NotificationTarget::Slack {
///         webhook_url: "https://hooks.slack.com/services/T/B/X".to_string(),
///         channel: "#deploys".to_string(),
///     }]),
///     EscalationPolicy::AutoReject,
/// ]);
/// assert_eq!(policy.len(), 2);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EscalationPolicy {
    /// Complete the step with `approved_by: "system:timeout"` and resume the run.
    AutoApprove,
    /// Fail the step and the run with the reason `"approval timeout"`.
    AutoReject,
    /// Notify each target without changing state, then restart the timer.
    Notify(Vec<NotificationTarget>),
    /// Reassign the approval to another user or group, then restart the timer.
    Escalate(String),
    /// Apply the policies one per expiry, in order.
    Chain(Vec<EscalationPolicy>),
}

/// Where an escalation notification is delivered.
///
/// Both variants are plain HTTP `POST`s, delivered with the engine's shared
/// retry/backoff configuration. A failed delivery is logged and never blocks
/// the timer reset.
///
/// # Examples
///
/// ```
/// use ironflow_engine::config::NotificationTarget;
///
/// let target = NotificationTarget::Webhook {
///     url: "https://example.com/sla".to_string(),
/// };
/// let json = serde_json::to_string(&target).expect("serialize");
/// assert!(json.contains("webhook"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationTarget {
    /// Plain HTTP `POST` of the escalation event as JSON.
    Webhook {
        /// Destination URL.
        url: String,
    },
    /// Slack incoming webhook; `channel` is echoed in the payload.
    Slack {
        /// Slack incoming-webhook URL.
        webhook_url: String,
        /// Channel name, echoed in the posted payload.
        channel: String,
    },
}

impl EscalationPolicy {
    /// The policy to apply at escalation stage `index`.
    ///
    /// For a [`Chain`](EscalationPolicy::Chain), this is the `index`-th link.
    /// For any other variant, stage `0` is the policy itself and every later
    /// stage is `None` — the chain is exhausted. A repeating policy never
    /// reaches stage 1 outside a chain because it re-arms at the same stage;
    /// see [`is_repeating`](Self::is_repeating).
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::EscalationPolicy;
    ///
    /// let single = EscalationPolicy::AutoReject;
    /// assert_eq!(single.stage(0), Some(&EscalationPolicy::AutoReject));
    /// assert_eq!(single.stage(1), None);
    ///
    /// let chain = EscalationPolicy::Chain(vec![
    ///     EscalationPolicy::Escalate("sre-oncall".to_string()),
    ///     EscalationPolicy::AutoReject,
    /// ]);
    /// assert_eq!(chain.stage(1), Some(&EscalationPolicy::AutoReject));
    /// assert_eq!(chain.stage(2), None);
    /// ```
    pub fn stage(&self, index: usize) -> Option<&EscalationPolicy> {
        match self {
            EscalationPolicy::Chain(policies) => policies.get(index),
            other => (index == 0).then_some(other),
        }
    }

    /// Whether this policy resolves the gate instead of leaving it open.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::EscalationPolicy;
    ///
    /// assert!(EscalationPolicy::AutoApprove.is_terminal());
    /// assert!(EscalationPolicy::AutoReject.is_terminal());
    /// assert!(!EscalationPolicy::Escalate("sre".to_string()).is_terminal());
    /// ```
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            EscalationPolicy::AutoApprove | EscalationPolicy::AutoReject
        )
    }

    /// Whether this policy re-arms the timer at the *same* stage.
    ///
    /// Outside a [`Chain`](EscalationPolicy::Chain), a repeating policy fires
    /// again at every expiry until a human resolves the gate. Inside a chain,
    /// the stage still advances so the next link eventually runs.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::EscalationPolicy;
    ///
    /// assert!(EscalationPolicy::Notify(Vec::new()).is_repeating());
    /// assert!(EscalationPolicy::Escalate("sre".to_string()).is_repeating());
    /// assert!(!EscalationPolicy::AutoApprove.is_repeating());
    /// ```
    pub fn is_repeating(&self) -> bool {
        matches!(
            self,
            EscalationPolicy::Notify(_) | EscalationPolicy::Escalate(_)
        )
    }

    /// Number of stages this policy declares.
    ///
    /// A [`Chain`](EscalationPolicy::Chain) reports its length; every other
    /// variant reports `1`.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::EscalationPolicy;
    ///
    /// assert_eq!(EscalationPolicy::AutoReject.len(), 1);
    /// assert_eq!(
    ///     EscalationPolicy::Chain(vec![EscalationPolicy::AutoReject]).len(),
    ///     1
    /// );
    /// ```
    pub fn len(&self) -> usize {
        match self {
            EscalationPolicy::Chain(policies) => policies.len(),
            _ => 1,
        }
    }

    /// Whether this policy declares no stage at all.
    ///
    /// Only an empty [`Chain`](EscalationPolicy::Chain) is empty: it never
    /// escalates, and the gate stays open with no timer after the first expiry.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::EscalationPolicy;
    ///
    /// assert!(EscalationPolicy::Chain(Vec::new()).is_empty());
    /// assert!(!EscalationPolicy::AutoReject.is_empty());
    /// ```
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slack() -> NotificationTarget {
        NotificationTarget::Slack {
            webhook_url: "https://hooks.slack.com/services/T/B/X".to_string(),
            channel: "#deploys".to_string(),
        }
    }

    #[test]
    fn auto_approve_serializes_as_a_bare_string() {
        let json = serde_json::to_string(&EscalationPolicy::AutoApprove).expect("serialize");
        assert_eq!(json, "\"auto_approve\"");
    }

    #[test]
    fn auto_reject_serializes_as_a_bare_string() {
        let json = serde_json::to_string(&EscalationPolicy::AutoReject).expect("serialize");
        assert_eq!(json, "\"auto_reject\"");
    }

    #[test]
    fn notify_is_externally_tagged() {
        let policy = EscalationPolicy::Notify(vec![NotificationTarget::Webhook {
            url: "https://example.com/sla".to_string(),
        }]);
        let json = serde_json::to_string(&policy).expect("serialize");
        assert!(json.starts_with("{\"notify\":["), "got {json}");

        let back: EscalationPolicy = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, policy);
    }

    #[test]
    fn escalate_is_externally_tagged() {
        let policy = EscalationPolicy::Escalate("sre-oncall".to_string());
        let json = serde_json::to_string(&policy).expect("serialize");
        assert_eq!(json, "{\"escalate\":\"sre-oncall\"}");

        let back: EscalationPolicy = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, policy);
    }

    #[test]
    fn chain_is_externally_tagged() {
        let policy = EscalationPolicy::Chain(vec![
            EscalationPolicy::Notify(vec![slack()]),
            EscalationPolicy::AutoReject,
        ]);
        let json = serde_json::to_string(&policy).expect("serialize");
        assert!(json.starts_with("{\"chain\":["), "got {json}");

        let back: EscalationPolicy = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, policy);
    }

    #[test]
    fn nested_chain_roundtrips() {
        let policy = EscalationPolicy::Chain(vec![
            EscalationPolicy::Chain(vec![EscalationPolicy::AutoApprove]),
            EscalationPolicy::AutoReject,
        ]);

        let json = serde_json::to_string(&policy).expect("serialize");
        let back: EscalationPolicy = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, policy);
    }

    #[test]
    fn notification_targets_roundtrip() {
        for target in [
            NotificationTarget::Webhook {
                url: "https://example.com/sla".to_string(),
            },
            slack(),
        ] {
            let json = serde_json::to_string(&target).expect("serialize");
            let back: NotificationTarget = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back, target);
        }
    }

    #[test]
    fn stage_of_a_single_policy_is_only_zero() {
        let policy = EscalationPolicy::Notify(vec![slack()]);
        assert_eq!(policy.stage(0), Some(&policy));
        assert!(policy.stage(1).is_none());
        assert!(policy.stage(99).is_none());
    }

    #[test]
    fn stage_walks_a_chain_in_order() {
        let policy = EscalationPolicy::Chain(vec![
            EscalationPolicy::Notify(vec![slack()]),
            EscalationPolicy::Escalate("sre-oncall".to_string()),
            EscalationPolicy::AutoReject,
        ]);

        assert_eq!(
            policy.stage(0),
            Some(&EscalationPolicy::Notify(vec![slack()]))
        );
        assert_eq!(
            policy.stage(1),
            Some(&EscalationPolicy::Escalate("sre-oncall".to_string()))
        );
        assert_eq!(policy.stage(2), Some(&EscalationPolicy::AutoReject));
        assert!(policy.stage(3).is_none());
    }

    #[test]
    fn terminal_and_repeating_are_mutually_exclusive() {
        let cases = [
            (EscalationPolicy::AutoApprove, true, false),
            (EscalationPolicy::AutoReject, true, false),
            (EscalationPolicy::Notify(vec![slack()]), false, true),
            (EscalationPolicy::Escalate("sre".to_string()), false, true),
            (EscalationPolicy::Chain(Vec::new()), false, false),
        ];

        for (policy, terminal, repeating) in cases {
            assert_eq!(policy.is_terminal(), terminal, "{policy:?}");
            assert_eq!(policy.is_repeating(), repeating, "{policy:?}");
        }
    }

    #[test]
    fn len_reports_chain_length_and_one_otherwise() {
        assert_eq!(EscalationPolicy::AutoApprove.len(), 1);
        assert_eq!(EscalationPolicy::Escalate("sre".to_string()).len(), 1);
        assert_eq!(
            EscalationPolicy::Chain(vec![
                EscalationPolicy::AutoApprove,
                EscalationPolicy::AutoReject
            ])
            .len(),
            2
        );
    }

    #[test]
    fn only_an_empty_chain_is_empty() {
        assert!(EscalationPolicy::Chain(Vec::new()).is_empty());
        assert!(!EscalationPolicy::Notify(Vec::new()).is_empty());
        assert!(!EscalationPolicy::AutoReject.is_empty());
    }
}
