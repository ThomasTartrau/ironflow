//! Dynamic approval requirement -- how many votes, and from whom, a gate needs.
//!
//! When an approval gate opens, the engine evaluates the approval rules of the
//! step in order and records the outcome as an [`ApprovalRequirement`] on the
//! step. Every vote cast on the gate is then appended as a [`StepApproval`].
//! The gate resolves once the number of distinct votes reaches
//! [`ApprovalRequirement::required_approvers`].

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Outcome of evaluating one approval rule when a gate opens.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::ApprovalRuleEvaluation;
///
/// let evaluation = ApprovalRuleEvaluation {
///     index: 0,
///     condition: "payload.amount > 10000".to_string(),
///     matched: true,
/// };
/// assert!(evaluation.matched);
/// ```
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApprovalRuleEvaluation {
    /// Position of the rule in the step configuration (0-based).
    pub index: u32,
    /// Source of the rule condition.
    pub condition: String,
    /// Whether the condition evaluated to `true`.
    pub matched: bool,
}

/// The approval requirement evaluated when a gate opened.
///
/// A step without approval rules carries no requirement at all; the default
/// value (one approval from anyone allowed to answer the gate) applies.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::ApprovalRequirement;
///
/// let requirement = ApprovalRequirement {
///     rule_index: Some(0),
///     condition: Some("payload.amount > 10000".to_string()),
///     required_approvers: 2,
///     approver_groups: vec!["finance".to_string()],
///     evaluated: Vec::new(),
/// };
/// assert!(!requirement.is_satisfied_by(1));
/// assert!(requirement.is_satisfied_by(2));
/// assert!(!requirement.allows_everyone());
/// ```
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApprovalRequirement {
    /// Index of the matched rule. `None` means no rule matched and the default
    /// requirement applies.
    pub rule_index: Option<u32>,
    /// Source of the matched rule condition.
    pub condition: Option<String>,
    /// Number of distinct approvals needed to resolve the gate.
    pub required_approvers: u32,
    /// Groups whose members may vote. Empty means anyone allowed to answer
    /// the gate may vote.
    #[serde(default)]
    pub approver_groups: Vec<String>,
    /// Rules evaluated in order, up to and including the matched one.
    #[serde(default)]
    pub evaluated: Vec<ApprovalRuleEvaluation>,
}

impl Default for ApprovalRequirement {
    /// One approval, from anyone, no matched rule.
    fn default() -> Self {
        Self {
            rule_index: None,
            condition: None,
            required_approvers: 1,
            approver_groups: Vec::new(),
            evaluated: Vec::new(),
        }
    }
}

impl ApprovalRequirement {
    /// Whether `received` distinct approvals are enough to resolve the gate.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_store::entities::ApprovalRequirement;
    ///
    /// let requirement = ApprovalRequirement::default();
    /// assert!(!requirement.is_satisfied_by(0));
    /// assert!(requirement.is_satisfied_by(1));
    /// ```
    pub fn is_satisfied_by(&self, received: usize) -> bool {
        received >= self.required_approvers as usize
    }

    /// Whether the requirement puts no group restriction on voters.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_store::entities::ApprovalRequirement;
    ///
    /// assert!(ApprovalRequirement::default().allows_everyone());
    /// ```
    pub fn allows_everyone(&self) -> bool {
        self.approver_groups.is_empty()
    }
}

/// One vote cast on an approval gate.
///
/// Votes are unique per [`user_id`](StepApproval::user_id): an API key votes as
/// its owner, and a user voting twice is ignored by the store.
///
/// # Examples
///
/// ```
/// use chrono::Utc;
/// use ironflow_store::entities::StepApproval;
/// use uuid::Uuid;
///
/// let vote = StepApproval {
///     user_id: Uuid::now_v7(),
///     approved_by: "alice".to_string(),
///     at: Utc::now(),
/// };
/// assert_eq!(vote.approved_by, "alice");
/// ```
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepApproval {
    /// The user who voted.
    pub user_id: Uuid,
    /// Display name of the voter at vote time.
    pub approved_by: String,
    /// When the vote was cast.
    pub at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use serde_json::{from_str, from_value, json, to_string};

    use super::*;

    fn requirement(required: u32) -> ApprovalRequirement {
        ApprovalRequirement {
            rule_index: Some(1),
            condition: Some("labels.env == \"production\"".to_string()),
            required_approvers: required,
            approver_groups: vec!["sre".to_string()],
            evaluated: vec![
                ApprovalRuleEvaluation {
                    index: 0,
                    condition: "payload.amount > 10000".to_string(),
                    matched: false,
                },
                ApprovalRuleEvaluation {
                    index: 1,
                    condition: "labels.env == \"production\"".to_string(),
                    matched: true,
                },
            ],
        }
    }

    #[test]
    fn requirement_serde_roundtrip() {
        let req = requirement(3);
        let json = to_string(&req).expect("serialize");
        let back: ApprovalRequirement = from_str(&json).expect("deserialize");
        assert_eq!(back, req);
    }

    #[test]
    fn requirement_defaults_missing_lists() {
        let back: ApprovalRequirement = from_value(json!({
            "rule_index": null,
            "condition": null,
            "required_approvers": 1
        }))
        .expect("deserialize");
        assert_eq!(back, ApprovalRequirement::default());
    }

    #[test]
    fn default_requires_one_approval_from_anyone() {
        let req = ApprovalRequirement::default();
        assert_eq!(req.rule_index, None);
        assert_eq!(req.condition, None);
        assert_eq!(req.required_approvers, 1);
        assert!(req.approver_groups.is_empty());
        assert!(req.evaluated.is_empty());
        assert!(req.allows_everyone());
    }

    #[test]
    fn is_satisfied_by_boundaries() {
        let req = requirement(3);
        assert!(!req.is_satisfied_by(0));
        assert!(!req.is_satisfied_by(2));
        assert!(req.is_satisfied_by(3));
        assert!(req.is_satisfied_by(4));
        assert!(!req.allows_everyone());
    }

    #[test]
    fn step_approval_serde_roundtrip() {
        let vote = StepApproval {
            user_id: Uuid::now_v7(),
            approved_by: "élodie".to_string(),
            at: Utc::now(),
        };
        let json = to_string(&vote).expect("serialize");
        let back: StepApproval = from_str(&json).expect("deserialize");
        assert_eq!(back, vote);
    }
}
