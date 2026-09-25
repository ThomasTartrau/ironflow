//! Approval requirement -- how many votes, and from whom, a gate needs.
//!
//! The workflow handler computes the approvers of a gate in Rust when the gate
//! opens; the engine records them as an [`ApprovalRequirement`] on the step.
//! Every vote cast on the gate is then appended as a [`StepApproval`]. The gate
//! resolves once the number of distinct votes reaches
//! [`ApprovalRequirement::required_approvers`].

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The approval requirement recorded when a gate opened.
///
/// A gate opened without approvers carries no requirement at all; the default
/// value (one approval from anyone allowed to answer the gate) applies.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::ApprovalRequirement;
///
/// let requirement = ApprovalRequirement {
///     reason: Some("amount > 10k".to_string()),
///     required_approvers: 2,
///     approver_groups: vec!["finance".to_string()],
/// };
/// assert!(!requirement.is_satisfied_by(1));
/// assert!(requirement.is_satisfied_by(2));
/// assert!(!requirement.allows_everyone());
/// ```
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApprovalRequirement {
    /// Why the handler asked for these approvers, for the audit trail. Never
    /// evaluated.
    #[serde(default)]
    pub reason: Option<String>,
    /// Number of distinct approvals needed to resolve the gate.
    pub required_approvers: u32,
    /// Groups whose members may vote. Empty means anyone allowed to answer
    /// the gate may vote.
    #[serde(default)]
    pub approver_groups: Vec<String>,
}

impl Default for ApprovalRequirement {
    /// One approval, from anyone, no reason.
    fn default() -> Self {
        Self {
            reason: None,
            required_approvers: 1,
            approver_groups: Vec::new(),
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
            reason: Some("production deploy".to_string()),
            required_approvers: required,
            approver_groups: vec!["sre".to_string()],
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
    fn requirement_defaults_missing_reason_and_groups() {
        let back: ApprovalRequirement =
            from_value(json!({"required_approvers": 1})).expect("deserialize");
        assert_eq!(back, ApprovalRequirement::default());
    }

    #[test]
    fn requirement_written_by_approval_rules_still_deserializes() {
        // Shape stored before approval rules were replaced by `Approvers`.
        let back: ApprovalRequirement = from_value(json!({
            "rule_index": 0,
            "condition": "payload.amount > 10000",
            "required_approvers": 2,
            "approver_groups": ["finance"],
            "evaluated": [{"index": 0, "condition": "payload.amount > 10000", "matched": true}]
        }))
        .expect("deserialize");
        assert_eq!(back.reason, None);
        assert_eq!(back.required_approvers, 2);
        assert_eq!(back.approver_groups, vec!["finance"]);
    }

    #[test]
    fn default_requires_one_approval_from_anyone() {
        let req = ApprovalRequirement::default();
        assert_eq!(req.reason, None);
        assert_eq!(req.required_approvers, 1);
        assert!(req.approver_groups.is_empty());
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
