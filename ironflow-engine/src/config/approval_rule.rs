//! [`ApprovalRule`] -- one row of the dynamic approval matrix.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::expression::Expression;

/// A conditional approval requirement.
///
/// When an approval gate opens, the rules of its
/// [`ApprovalConfig`](super::ApprovalConfig) are evaluated in order against the
/// run context; the first rule whose [`condition`](Self::condition) holds
/// decides how many distinct approvals the gate needs and, optionally, which
/// groups the approvers must belong to. See [`Expression`] for the condition
/// syntax.
///
/// Deserialization rejects an invalid condition and `required_approvers == 0`.
///
/// # Examples
///
/// ```
/// use ironflow_engine::config::ApprovalRule;
/// use serde_json::json;
///
/// let rule = ApprovalRule::new("payload.amount > 10000", 2).with_approver_groups(["finance"]);
/// assert_eq!(rule.required_approvers(), 2);
/// assert_eq!(rule.approver_groups(), ["finance".to_string()]);
/// assert!(rule.matches(&json!({"payload": {"amount": 15000}})));
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RawApprovalRule")]
pub struct ApprovalRule {
    condition: Expression,
    required_approvers: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    approver_groups: Vec<String>,
}

/// Unvalidated wire form of an [`ApprovalRule`].
#[derive(Deserialize)]
struct RawApprovalRule {
    condition: Expression,
    required_approvers: usize,
    #[serde(default)]
    approver_groups: Vec<String>,
}

impl TryFrom<RawApprovalRule> for ApprovalRule {
    type Error = String;

    fn try_from(raw: RawApprovalRule) -> Result<Self, Self::Error> {
        if raw.required_approvers == 0 {
            return Err(ZERO_APPROVERS.to_string());
        }
        Ok(Self {
            condition: raw.condition,
            required_approvers: raw.required_approvers,
            approver_groups: normalize_groups(raw.approver_groups)?,
        })
    }
}

const ZERO_APPROVERS: &str = "required_approvers must be greater than zero";
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

impl ApprovalRule {
    /// Create a rule from a condition source and a number of approvers.
    ///
    /// # Panics
    ///
    /// Panics if `condition` is not a valid [`Expression`] or if
    /// `required_approvers` is zero.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::ApprovalRule;
    ///
    /// let rule = ApprovalRule::new("labels.env == 'production'", 2);
    /// assert_eq!(rule.condition().source(), "labels.env == 'production'");
    /// ```
    pub fn new(condition: &str, required_approvers: usize) -> Self {
        let condition = Expression::parse(condition)
            .unwrap_or_else(|err| panic!("invalid approval rule condition: {err}"));
        Self::from_expression(condition, required_approvers)
    }

    /// Create a rule from an already parsed [`Expression`].
    ///
    /// # Panics
    ///
    /// Panics if `required_approvers` is zero.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::ApprovalRule;
    /// use ironflow_engine::expression::Expression;
    ///
    /// # fn main() -> Result<(), ironflow_engine::expression::ExpressionError> {
    /// let rule = ApprovalRule::from_expression(Expression::parse("payload.urgent")?, 3);
    /// assert_eq!(rule.required_approvers(), 3);
    /// # Ok(())
    /// # }
    /// ```
    pub fn from_expression(condition: Expression, required_approvers: usize) -> Self {
        assert!(required_approvers > 0, "{ZERO_APPROVERS}");
        Self {
            condition,
            required_approvers,
            approver_groups: Vec::new(),
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
    /// use ironflow_engine::config::ApprovalRule;
    ///
    /// let rule = ApprovalRule::new("payload.amount > 10000", 2)
    ///     .with_approver_groups([" finance ", "legal", "finance"]);
    /// assert_eq!(rule.approver_groups(), ["finance".to_string(), "legal".to_string()]);
    /// ```
    pub fn with_approver_groups<I, S>(mut self, groups: I) -> Self
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

    /// The condition deciding whether this rule applies.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::ApprovalRule;
    ///
    /// let rule = ApprovalRule::new("payload.urgent", 1);
    /// assert_eq!(rule.condition().to_string(), "payload.urgent");
    /// ```
    pub fn condition(&self) -> &Expression {
        &self.condition
    }

    /// Number of distinct approvals the gate needs when this rule applies.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::ApprovalRule;
    ///
    /// assert_eq!(ApprovalRule::new("payload.urgent", 3).required_approvers(), 3);
    /// ```
    pub fn required_approvers(&self) -> usize {
        self.required_approvers
    }

    /// Groups whose members may vote. Empty means no group restriction.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::ApprovalRule;
    ///
    /// assert!(ApprovalRule::new("payload.urgent", 1).approver_groups().is_empty());
    /// ```
    pub fn approver_groups(&self) -> &[String] {
        &self.approver_groups
    }

    /// Whether the condition holds for the given context.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::ApprovalRule;
    /// use serde_json::json;
    ///
    /// let rule = ApprovalRule::new("labels.env == 'production'", 2);
    /// assert!(rule.matches(&json!({"labels": {"env": "production"}})));
    /// assert!(!rule.matches(&json!({"labels": {}})));
    /// ```
    pub fn matches(&self, ctx: &Value) -> bool {
        self.condition.evaluate(ctx)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{from_value, json, to_value};

    use super::*;

    #[test]
    fn new_parses_the_condition() {
        let rule = ApprovalRule::new("payload.amount > 10000", 2);
        assert_eq!(rule.condition().source(), "payload.amount > 10000");
        assert_eq!(rule.required_approvers(), 2);
        assert!(rule.approver_groups().is_empty());
    }

    #[test]
    fn from_expression_keeps_the_expression() {
        let expr = Expression::parse("labels.env == 'prod'").expect("parse");
        let rule = ApprovalRule::from_expression(expr.clone(), 1);
        assert_eq!(rule.condition(), &expr);
    }

    #[test]
    fn matches_evaluates_the_condition() {
        let rule = ApprovalRule::new("payload.amount > 10000", 2);
        assert!(rule.matches(&json!({"payload": {"amount": 15000}})));
        assert!(!rule.matches(&json!({"payload": {"amount": 10}})));
        assert!(!rule.matches(&json!({})));
    }

    #[test]
    fn approver_groups_are_trimmed_and_deduplicated() {
        let rule = ApprovalRule::new("payload.urgent", 1)
            .with_approver_groups(vec![" sre ", "finance", "sre", "finance "]);
        assert_eq!(
            rule.approver_groups(),
            ["sre".to_string(), "finance".to_string()]
        );
    }

    #[test]
    #[should_panic(expected = "invalid approval rule condition")]
    fn new_rejects_an_invalid_condition() {
        let _ = ApprovalRule::new("foo.bar == 1", 1);
    }

    #[test]
    #[should_panic(expected = "required_approvers must be greater than zero")]
    fn new_rejects_zero_approvers() {
        let _ = ApprovalRule::new("payload.urgent", 0);
    }

    #[test]
    #[should_panic(expected = "approver group must not be empty")]
    fn with_approver_groups_rejects_a_blank_group() {
        let _ = ApprovalRule::new("payload.urgent", 1).with_approver_groups(["finance", "  "]);
    }

    #[test]
    fn serde_roundtrip() {
        let rule = ApprovalRule::new("payload.amount > 10000", 2).with_approver_groups(["finance"]);
        let json = to_value(&rule).expect("serialize");
        assert_eq!(
            json,
            json!({
                "condition": "payload.amount > 10000",
                "required_approvers": 2,
                "approver_groups": ["finance"],
            })
        );
        let back: ApprovalRule = from_value(json).expect("deserialize");
        assert_eq!(back, rule);
    }

    #[test]
    fn serde_omits_empty_groups() {
        let rule = ApprovalRule::new("payload.urgent", 1);
        let json = to_value(&rule).expect("serialize");
        assert!(json.get("approver_groups").is_none());
        let back: ApprovalRule = from_value(json).expect("deserialize");
        assert_eq!(back, rule);
    }

    #[test]
    fn serde_rejects_zero_approvers() {
        let err = from_value::<ApprovalRule>(json!({
            "condition": "payload.urgent",
            "required_approvers": 0,
        }))
        .expect_err("zero approvers");
        assert!(err.to_string().contains("greater than zero"));
    }

    #[test]
    fn serde_rejects_an_invalid_condition() {
        let err = from_value::<ApprovalRule>(json!({
            "condition": "foo.bar",
            "required_approvers": 1,
        }))
        .expect_err("invalid condition");
        assert!(err.to_string().contains("unknown root"));
    }

    #[test]
    fn serde_rejects_a_blank_group() {
        let err = from_value::<ApprovalRule>(json!({
            "condition": "payload.urgent",
            "required_approvers": 1,
            "approver_groups": [" "],
        }))
        .expect_err("blank group");
        assert!(err.to_string().contains("must not be empty"));
    }
}
