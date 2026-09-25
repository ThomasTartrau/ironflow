//! [`Approvers`] -- who must approve a gate, and how many of them.

use serde::{Deserialize, Serialize};

use ironflow_store::entities::ApprovalRequirement;

/// The approvers an approval gate requires.
///
/// The workflow handler computes them in plain Rust, from its typed input and
/// the outputs of earlier steps, and passes them to
/// [`ApprovalConfig::requiring`](super::ApprovalConfig::requiring). The engine
/// records them on the gate as an [`ApprovalRequirement`] when it opens; that
/// record stays the source of truth on replay and resume.
///
/// Deserialization rejects zero approvers and a blank group name.
///
/// # Examples
///
/// ```
/// use ironflow_engine::config::Approvers;
///
/// let amount = 15_000;
/// let approvers = match amount {
///     a if a > 100_000 => Approvers::at_least(3)
///         .from_groups(["finance", "board"])
///         .because("amount > 100k"),
///     a if a > 10_000 => Approvers::at_least(2)
///         .from_groups(["finance"])
///         .because("amount > 10k"),
///     _ => Approvers::any(),
/// };
///
/// assert_eq!(approvers.required(), 2);
/// assert_eq!(approvers.groups(), ["finance".to_string()]);
/// assert_eq!(approvers.reason(), Some("amount > 10k"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawApprovers")]
pub struct Approvers {
    required_approvers: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    approver_groups: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

/// Unvalidated wire form of [`Approvers`].
#[derive(Deserialize)]
struct RawApprovers {
    required_approvers: u32,
    #[serde(default)]
    approver_groups: Vec<String>,
    #[serde(default)]
    reason: Option<String>,
}

impl TryFrom<RawApprovers> for Approvers {
    type Error = String;

    fn try_from(raw: RawApprovers) -> Result<Self, Self::Error> {
        if raw.required_approvers == 0 {
            return Err(ZERO_APPROVERS.to_string());
        }
        Ok(Self {
            required_approvers: raw.required_approvers,
            approver_groups: normalize_groups(raw.approver_groups)?,
            reason: raw.reason,
        })
    }
}

const ZERO_APPROVERS: &str = "an approval gate needs at least one approver";
const BLANK_GROUP: &str = "approver group must not be empty";

/// Trim and deduplicate group names, keeping the first occurrence order.
fn normalize_groups<I, S>(groups: I) -> Result<Vec<String>, String>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut normalized: Vec<String> = Vec::new();
    for group in groups {
        let group = group.into().trim().to_string();
        if group.is_empty() {
            return Err(BLANK_GROUP.to_string());
        }
        if !normalized.contains(&group) {
            normalized.push(group);
        }
    }
    Ok(normalized)
}

impl Approvers {
    /// One approval, from anyone allowed to answer the gate.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::Approvers;
    ///
    /// let approvers = Approvers::any();
    /// assert_eq!(approvers.required(), 1);
    /// assert!(approvers.groups().is_empty());
    /// assert_eq!(approvers.reason(), None);
    /// ```
    pub fn any() -> Self {
        Self {
            required_approvers: 1,
            approver_groups: Vec::new(),
            reason: None,
        }
    }

    /// At least `count` distinct approvals.
    ///
    /// # Panics
    ///
    /// Panics if `count` is zero: a gate nobody has to approve is not a gate.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::Approvers;
    ///
    /// assert_eq!(Approvers::at_least(3).required(), 3);
    /// ```
    ///
    /// ```should_panic
    /// use ironflow_engine::config::Approvers;
    ///
    /// let _ = Approvers::at_least(0);
    /// ```
    pub fn at_least(count: u32) -> Self {
        assert!(count > 0, "{ZERO_APPROVERS}");
        Self {
            required_approvers: count,
            ..Self::any()
        }
    }

    /// Restrict voting to members of the given groups.
    ///
    /// Names are trimmed and deduplicated. Admins may always vote.
    ///
    /// # Panics
    ///
    /// Panics if a group name is empty or only whitespace.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::Approvers;
    ///
    /// let approvers = Approvers::at_least(2).from_groups([" finance ", "legal", "finance"]);
    /// assert_eq!(approvers.groups(), ["finance".to_string(), "legal".to_string()]);
    /// ```
    pub fn from_groups<I, S>(mut self, groups: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        match normalize_groups(groups) {
            Ok(groups) => self.approver_groups = groups,
            Err(err) => panic!("{err}"),
        }
        self
    }

    /// Record why these approvers are required.
    ///
    /// The reason is an audit label shown on the dashboard and carried by the
    /// approval events. It is never evaluated.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::Approvers;
    ///
    /// let approvers = Approvers::at_least(2).because("amount > 10k");
    /// assert_eq!(approvers.reason(), Some("amount > 10k"));
    /// ```
    pub fn because(mut self, reason: &str) -> Self {
        self.reason = Some(reason.to_string());
        self
    }

    /// Number of distinct approvals the gate needs.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::Approvers;
    ///
    /// assert_eq!(Approvers::any().required(), 1);
    /// ```
    pub fn required(&self) -> u32 {
        self.required_approvers
    }

    /// Groups whose members may vote. Empty means no group restriction.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::Approvers;
    ///
    /// assert!(Approvers::at_least(2).groups().is_empty());
    /// ```
    pub fn groups(&self) -> &[String] {
        &self.approver_groups
    }

    /// Why these approvers are required, if the handler said so.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::Approvers;
    ///
    /// assert_eq!(Approvers::any().because("routine").reason(), Some("routine"));
    /// ```
    pub fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }

    /// The requirement the engine records on the gate.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::{ApprovalRequirement, Approvers};
    ///
    /// let requirement = Approvers::at_least(2).from_groups(["finance"]).to_requirement();
    /// assert_eq!(requirement.required_approvers, 2);
    /// assert_eq!(requirement.approver_groups, vec!["finance".to_string()]);
    /// assert_eq!(Approvers::any().to_requirement(), ApprovalRequirement::default());
    /// ```
    pub fn to_requirement(&self) -> ApprovalRequirement {
        ApprovalRequirement {
            reason: self.reason.clone(),
            required_approvers: self.required_approvers,
            approver_groups: self.approver_groups.clone(),
        }
    }
}

impl Default for Approvers {
    /// Same as [`Approvers::any`].
    fn default() -> Self {
        Self::any()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{from_value, json, to_value};

    use super::*;

    #[test]
    fn any_requires_one_approval_from_anyone() {
        let approvers = Approvers::any();
        assert_eq!(approvers.required(), 1);
        assert!(approvers.groups().is_empty());
        assert_eq!(approvers.reason(), None);
        assert_eq!(approvers, Approvers::default());
    }

    #[test]
    fn at_least_sets_the_count() {
        assert_eq!(Approvers::at_least(4).required(), 4);
    }

    #[test]
    #[should_panic(expected = "an approval gate needs at least one approver")]
    fn at_least_rejects_zero() {
        let _ = Approvers::at_least(0);
    }

    #[test]
    fn from_groups_trims_and_deduplicates() {
        let approvers =
            Approvers::at_least(1).from_groups(vec![" sre ", "finance", "sre", "finance "]);
        assert_eq!(
            approvers.groups(),
            ["sre".to_string(), "finance".to_string()]
        );
    }

    #[test]
    fn from_groups_keeps_unicode_names() {
        let approvers = Approvers::any().from_groups(["équipe-sécurité"]);
        assert_eq!(approvers.groups(), ["équipe-sécurité".to_string()]);
    }

    #[test]
    #[should_panic(expected = "approver group must not be empty")]
    fn from_groups_rejects_a_blank_group() {
        let _ = Approvers::at_least(1).from_groups(["finance", "  "]);
    }

    #[test]
    fn because_sets_the_reason() {
        let approvers = Approvers::at_least(2).because("amount > 10k");
        assert_eq!(approvers.reason(), Some("amount > 10k"));
    }

    #[test]
    fn to_requirement_copies_every_field() {
        let requirement = Approvers::at_least(3)
            .from_groups(["finance", "board"])
            .because("amount > 100k")
            .to_requirement();
        assert_eq!(
            requirement,
            ApprovalRequirement {
                reason: Some("amount > 100k".to_string()),
                required_approvers: 3,
                approver_groups: vec!["finance".to_string(), "board".to_string()],
            }
        );
    }

    #[test]
    fn serde_roundtrip() {
        let approvers = Approvers::at_least(2)
            .from_groups(["finance"])
            .because("amount > 10k");
        let json = to_value(&approvers).expect("serialize");
        assert_eq!(
            json,
            json!({
                "required_approvers": 2,
                "approver_groups": ["finance"],
                "reason": "amount > 10k",
            })
        );
        let back: Approvers = from_value(json).expect("deserialize");
        assert_eq!(back, approvers);
    }

    #[test]
    fn serde_omits_empty_groups_and_reason() {
        let json = to_value(Approvers::any()).expect("serialize");
        assert_eq!(json, json!({"required_approvers": 1}));
    }

    #[test]
    fn serde_rejects_zero_approvers() {
        let err =
            from_value::<Approvers>(json!({"required_approvers": 0})).expect_err("zero approvers");
        assert!(err.to_string().contains("at least one approver"));
    }

    #[test]
    fn serde_rejects_a_blank_group() {
        let err = from_value::<Approvers>(json!({
            "required_approvers": 1,
            "approver_groups": [" "],
        }))
        .expect_err("blank group");
        assert!(err.to_string().contains("must not be empty"));
    }
}
