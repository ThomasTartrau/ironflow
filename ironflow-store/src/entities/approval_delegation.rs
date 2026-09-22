//! Approval delegation entity -- who may answer a gate on someone else's behalf.
//!
//! A delegation hands the approval power of one user (the delegator) to another
//! (the delegate) for a bounded time window, optionally narrowed to the
//! workflows whose name matches a glob. It exists so an absent approver never
//! blocks every run waiting on their gate.

use chrono::{DateTime, Utc};
use glob::{Pattern, PatternError};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

/// A persisted delegation of approval power between two users.
///
/// The window is half-open: `valid_from` is inclusive, `valid_until` is
/// exclusive. Expired rows are never deleted, they are simply filtered out at
/// read time by
/// [`ApprovalDelegationStore::list_active_delegations`](crate::approval_delegation_store::ApprovalDelegationStore::list_active_delegations).
///
/// # Examples
///
/// ```
/// use chrono::{TimeDelta, Utc};
/// use ironflow_store::entities::ApprovalDelegation;
/// use uuid::Uuid;
///
/// let now = Utc::now();
/// let delegation = ApprovalDelegation {
///     id: Uuid::now_v7(),
///     from_user_id: Uuid::now_v7(),
///     to_user_id: Uuid::now_v7(),
///     valid_from: now,
///     valid_until: now + TimeDelta::days(7),
///     workflow_filter: Some("deploy-*".to_string()),
///     created_at: now,
/// };
/// assert!(delegation.covers("deploy-prod", now));
/// assert!(!delegation.covers("cleanup", now));
/// ```
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalDelegation {
    /// Unique delegation ID (UUID v7).
    pub id: Uuid,
    /// User handing over their approval power.
    pub from_user_id: Uuid,
    /// User receiving the approval power.
    pub to_user_id: Uuid,
    /// Start of the validity window (inclusive).
    pub valid_from: DateTime<Utc>,
    /// End of the validity window (exclusive).
    pub valid_until: DateTime<Utc>,
    /// Glob on the workflow name, e.g. `"deploy-*"`. `None` matches every workflow.
    pub workflow_filter: Option<String>,
    /// When the delegation was created.
    pub created_at: DateTime<Utc>,
}

impl ApprovalDelegation {
    /// Whether `now` falls inside the validity window.
    ///
    /// The window is half-open: a delegation is already active at exactly
    /// `valid_from` and already inactive at exactly `valid_until`.
    ///
    /// # Examples
    ///
    /// ```
    /// use chrono::{TimeDelta, Utc};
    /// use ironflow_store::entities::ApprovalDelegation;
    /// use uuid::Uuid;
    ///
    /// let start = Utc::now();
    /// let end = start + TimeDelta::hours(1);
    /// let delegation = ApprovalDelegation {
    ///     id: Uuid::now_v7(),
    ///     from_user_id: Uuid::now_v7(),
    ///     to_user_id: Uuid::now_v7(),
    ///     valid_from: start,
    ///     valid_until: end,
    ///     workflow_filter: None,
    ///     created_at: start,
    /// };
    /// assert!(delegation.is_active_at(start));
    /// assert!(!delegation.is_active_at(end));
    /// ```
    pub fn is_active_at(&self, now: DateTime<Utc>) -> bool {
        self.valid_from <= now && now < self.valid_until
    }

    /// Whether the delegation applies to the workflow named `workflow_name`.
    ///
    /// A `None` filter matches every workflow. An unparseable pattern matches
    /// nothing: a corrupted row can never widen someone's power.
    ///
    /// # Examples
    ///
    /// ```
    /// use chrono::Utc;
    /// use ironflow_store::entities::ApprovalDelegation;
    /// use uuid::Uuid;
    ///
    /// let now = Utc::now();
    /// let delegation = ApprovalDelegation {
    ///     id: Uuid::now_v7(),
    ///     from_user_id: Uuid::now_v7(),
    ///     to_user_id: Uuid::now_v7(),
    ///     valid_from: now,
    ///     valid_until: now,
    ///     workflow_filter: Some("deploy-*".to_string()),
    ///     created_at: now,
    /// };
    /// assert!(delegation.matches_workflow("deploy-prod"));
    /// assert!(!delegation.matches_workflow("cleanup"));
    /// ```
    pub fn matches_workflow(&self, workflow_name: &str) -> bool {
        match &self.workflow_filter {
            None => true,
            Some(filter) => Pattern::new(filter)
                .map(|p| p.matches(workflow_name))
                .unwrap_or(false),
        }
    }

    /// Whether the delegation is active at `now` **and** covers `workflow_name`.
    ///
    /// # Examples
    ///
    /// ```
    /// use chrono::{TimeDelta, Utc};
    /// use ironflow_store::entities::ApprovalDelegation;
    /// use uuid::Uuid;
    ///
    /// let now = Utc::now();
    /// let delegation = ApprovalDelegation {
    ///     id: Uuid::now_v7(),
    ///     from_user_id: Uuid::now_v7(),
    ///     to_user_id: Uuid::now_v7(),
    ///     valid_from: now - TimeDelta::hours(1),
    ///     valid_until: now + TimeDelta::hours(1),
    ///     workflow_filter: None,
    ///     created_at: now,
    /// };
    /// assert!(delegation.covers("anything", now));
    /// ```
    pub fn covers(&self, workflow_name: &str, now: DateTime<Utc>) -> bool {
        self.is_active_at(now) && self.matches_workflow(workflow_name)
    }
}

/// Parameters for creating a new approval delegation.
///
/// # Examples
///
/// ```
/// use chrono::{TimeDelta, Utc};
/// use ironflow_store::entities::NewApprovalDelegation;
/// use uuid::Uuid;
///
/// let now = Utc::now();
/// let new = NewApprovalDelegation {
///     from_user_id: Uuid::now_v7(),
///     to_user_id: Uuid::now_v7(),
///     valid_from: now,
///     valid_until: now + TimeDelta::days(3),
///     workflow_filter: None,
/// };
/// assert!(new.workflow_filter.is_none());
/// ```
#[derive(Debug, Clone)]
pub struct NewApprovalDelegation {
    /// User handing over their approval power.
    pub from_user_id: Uuid,
    /// User receiving the approval power.
    pub to_user_id: Uuid,
    /// Start of the validity window (inclusive).
    pub valid_from: DateTime<Utc>,
    /// End of the validity window (exclusive).
    pub valid_until: DateTime<Utc>,
    /// Glob on the workflow name. `None` matches every workflow.
    pub workflow_filter: Option<String>,
}

/// Filter criteria for listing delegations.
///
/// Every field is optional; a `None` field applies no constraint. Set fields
/// are combined with AND.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::DelegationFilter;
/// use uuid::Uuid;
///
/// let filter = DelegationFilter {
///     to_user_id: Some(Uuid::now_v7()),
///     ..DelegationFilter::default()
/// };
/// assert!(filter.from_user_id.is_none());
/// ```
#[derive(Debug, Clone, Default)]
pub struct DelegationFilter {
    /// Only delegations granted by this user.
    pub from_user_id: Option<Uuid>,
    /// Only delegations received by this user.
    pub to_user_id: Option<Uuid>,
    /// Only delegations this user granted or received.
    pub involving_user_id: Option<Uuid>,
}

/// Why a workflow filter was refused.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::WorkflowFilterError;
///
/// let err = WorkflowFilterError::Empty;
/// assert_eq!(err.to_string(), "workflow filter must not be empty");
/// ```
#[derive(Debug, Error)]
pub enum WorkflowFilterError {
    /// The filter was empty or whitespace only.
    #[error("workflow filter must not be empty")]
    Empty,
    /// The filter is not a valid glob pattern.
    #[error("invalid glob pattern: {0}")]
    Invalid(#[from] PatternError),
}

/// Validate a workflow filter before persisting it.
///
/// Rejects an empty or whitespace-only string, then checks the glob syntax.
/// Validating at write time keeps
/// [`ApprovalDelegation::matches_workflow`] from silently matching nothing.
///
/// # Errors
///
/// Returns [`WorkflowFilterError::Empty`] when `pattern` holds no
/// non-whitespace character, and [`WorkflowFilterError::Invalid`] when it is
/// not a valid glob.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::validate_workflow_filter;
///
/// assert!(validate_workflow_filter("deploy-*").is_ok());
/// assert!(validate_workflow_filter("").is_err());
/// assert!(validate_workflow_filter("[unclosed").is_err());
/// ```
pub fn validate_workflow_filter(pattern: &str) -> Result<(), WorkflowFilterError> {
    if pattern.trim().is_empty() {
        return Err(WorkflowFilterError::Empty);
    }
    Pattern::new(pattern)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::TimeDelta;
    use serde_json::{from_str, to_string};

    use super::*;

    fn delegation(valid_from: DateTime<Utc>, valid_until: DateTime<Utc>) -> ApprovalDelegation {
        ApprovalDelegation {
            id: Uuid::now_v7(),
            from_user_id: Uuid::now_v7(),
            to_user_id: Uuid::now_v7(),
            valid_from,
            valid_until,
            workflow_filter: None,
            created_at: valid_from,
        }
    }

    #[test]
    fn active_window_includes_its_start_and_excludes_its_end() {
        let start = Utc::now();
        let end = start + TimeDelta::hours(2);
        let d = delegation(start, end);

        assert!(d.is_active_at(start), "valid_from is inclusive");
        assert!(d.is_active_at(start + TimeDelta::hours(1)));
        assert!(!d.is_active_at(end), "valid_until is exclusive");
    }

    #[test]
    fn a_delegation_is_inactive_before_and_after_its_window() {
        let start = Utc::now();
        let end = start + TimeDelta::hours(2);
        let d = delegation(start, end);

        assert!(!d.is_active_at(start - TimeDelta::seconds(1)));
        assert!(!d.is_active_at(end + TimeDelta::seconds(1)));
    }

    #[test]
    fn no_filter_matches_every_workflow() {
        let now = Utc::now();
        let d = delegation(now, now + TimeDelta::hours(1));

        assert!(d.matches_workflow("deploy-prod"));
        assert!(d.matches_workflow("cleanup"));
        assert!(d.matches_workflow(""));
    }

    #[test]
    fn glob_filter_matches_only_its_prefix() {
        let now = Utc::now();
        let d = ApprovalDelegation {
            workflow_filter: Some("deploy-*".to_string()),
            ..delegation(now, now + TimeDelta::hours(1))
        };

        assert!(d.matches_workflow("deploy-prod"));
        assert!(d.matches_workflow("deploy-"));
        assert!(!d.matches_workflow("cleanup"));
        assert!(!d.matches_workflow("redeploy-prod"));
    }

    #[test]
    fn star_filter_matches_every_workflow() {
        let now = Utc::now();
        let d = ApprovalDelegation {
            workflow_filter: Some("*".to_string()),
            ..delegation(now, now + TimeDelta::hours(1))
        };

        assert!(d.matches_workflow("deploy-prod"));
        assert!(d.matches_workflow("cleanup"));
    }

    #[test]
    fn an_invalid_pattern_matches_nothing() {
        let now = Utc::now();
        let d = ApprovalDelegation {
            workflow_filter: Some("[unclosed".to_string()),
            ..delegation(now, now + TimeDelta::hours(1))
        };

        assert!(!d.matches_workflow("deploy-prod"));
        assert!(!d.matches_workflow("[unclosed"));
    }

    #[test]
    fn covers_combines_the_window_and_the_filter() {
        let now = Utc::now();
        let d = ApprovalDelegation {
            workflow_filter: Some("deploy-*".to_string()),
            ..delegation(now - TimeDelta::hours(1), now + TimeDelta::hours(1))
        };

        assert!(d.covers("deploy-prod", now));
        assert!(!d.covers("cleanup", now), "workflow outside the filter");
        assert!(
            !d.covers("deploy-prod", now + TimeDelta::hours(2)),
            "instant outside the window"
        );
    }

    #[test]
    fn approval_delegation_serde_roundtrip() {
        let now = Utc::now();
        let d = ApprovalDelegation {
            workflow_filter: Some("deploy-*".to_string()),
            ..delegation(now, now + TimeDelta::days(1))
        };

        let json = to_string(&d).expect("serialize");
        let back: ApprovalDelegation = from_str(&json).expect("deserialize");

        assert_eq!(back.id, d.id);
        assert_eq!(back.from_user_id, d.from_user_id);
        assert_eq!(back.to_user_id, d.to_user_id);
        assert_eq!(back.workflow_filter, d.workflow_filter);
        assert_eq!(back.valid_until, d.valid_until);
    }

    #[test]
    fn validate_workflow_filter_accepts_a_glob() {
        assert!(validate_workflow_filter("deploy-*").is_ok());
        assert!(validate_workflow_filter("*").is_ok());
        assert!(validate_workflow_filter("deploy").is_ok());
    }

    #[test]
    fn validate_workflow_filter_rejects_a_blank_pattern() {
        assert!(matches!(
            validate_workflow_filter(""),
            Err(WorkflowFilterError::Empty)
        ));
        assert!(matches!(
            validate_workflow_filter("   "),
            Err(WorkflowFilterError::Empty)
        ));
    }

    #[test]
    fn validate_workflow_filter_rejects_a_broken_glob() {
        assert!(matches!(
            validate_workflow_filter("[unclosed"),
            Err(WorkflowFilterError::Invalid(_))
        ));
    }

    #[test]
    fn delegation_filter_defaults_to_no_constraint() {
        let filter = DelegationFilter::default();
        assert!(filter.from_user_id.is_none());
        assert!(filter.to_user_id.is_none());
        assert!(filter.involving_user_id.is_none());
    }
}
