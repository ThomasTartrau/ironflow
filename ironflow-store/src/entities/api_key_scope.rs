//! Scopes for API key permissions.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Permission scope for an API key.
///
/// Each scope grants access to a specific set of actions.
/// A key with no scopes has no permissions.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiKeyScope {
    /// Read workflow definitions.
    WorkflowsRead,
    /// Read runs and their steps.
    RunsRead,
    /// Create new runs (trigger workflows).
    RunsWrite,
    /// Cancel, approve, reject, retry runs.
    RunsManage,
    /// Read aggregated statistics.
    StatsRead,
    /// Full access to all operations.
    Admin,
}

impl ApiKeyScope {
    /// Check whether this scope grants the required permission.
    pub fn permits(&self, required: &ApiKeyScope) -> bool {
        match self {
            ApiKeyScope::Admin => true,
            other => other == required,
        }
    }

    /// Check whether a set of scopes grants the required permission.
    pub fn has_permission(scopes: &[ApiKeyScope], required: &ApiKeyScope) -> bool {
        scopes.iter().any(|s| s.permits(required))
    }

    /// All available scopes (excluding admin).
    pub fn all_non_admin() -> Vec<ApiKeyScope> {
        vec![
            ApiKeyScope::WorkflowsRead,
            ApiKeyScope::RunsRead,
            ApiKeyScope::RunsWrite,
            ApiKeyScope::RunsManage,
            ApiKeyScope::StatsRead,
        ]
    }

    /// Scopes a non-admin member is allowed to use.
    ///
    /// Whitelist approach: only these scopes are permitted for members.
    /// Any scope not listed here is forbidden for non-admin users.
    pub fn member_allowed() -> &'static [ApiKeyScope] {
        &[
            ApiKeyScope::WorkflowsRead,
            ApiKeyScope::RunsRead,
            ApiKeyScope::StatsRead,
        ]
    }

    /// Check whether all scopes in the set are allowed for a non-admin member.
    pub fn all_allowed_for_member(scopes: &[ApiKeyScope]) -> bool {
        let allowed = Self::member_allowed();
        scopes.iter().all(|s| allowed.contains(s))
    }
}

impl fmt::Display for ApiKeyScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            ApiKeyScope::WorkflowsRead => "workflows_read",
            ApiKeyScope::RunsRead => "runs_read",
            ApiKeyScope::RunsWrite => "runs_write",
            ApiKeyScope::RunsManage => "runs_manage",
            ApiKeyScope::StatsRead => "stats_read",
            ApiKeyScope::Admin => "admin",
        };
        f.write_str(s)
    }
}

/// Error when parsing an invalid scope string.
#[derive(Debug, Clone)]
pub struct InvalidScope(pub String);

impl fmt::Display for InvalidScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid API key scope: {}", self.0)
    }
}

impl std::error::Error for InvalidScope {}

impl FromStr for ApiKeyScope {
    type Err = InvalidScope;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "workflows_read" => Ok(ApiKeyScope::WorkflowsRead),
            "runs_read" => Ok(ApiKeyScope::RunsRead),
            "runs_write" => Ok(ApiKeyScope::RunsWrite),
            "runs_manage" => Ok(ApiKeyScope::RunsManage),
            "stats_read" => Ok(ApiKeyScope::StatsRead),
            "admin" => Ok(ApiKeyScope::Admin),
            _ => Err(InvalidScope(s.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admin_permits_everything() {
        let admin = ApiKeyScope::Admin;
        assert!(admin.permits(&ApiKeyScope::RunsRead));
        assert!(admin.permits(&ApiKeyScope::RunsWrite));
        assert!(admin.permits(&ApiKeyScope::RunsManage));
        assert!(admin.permits(&ApiKeyScope::WorkflowsRead));
        assert!(admin.permits(&ApiKeyScope::StatsRead));
        assert!(admin.permits(&ApiKeyScope::Admin));
    }

    #[test]
    fn regular_scope_only_permits_itself() {
        let scope = ApiKeyScope::RunsRead;
        assert!(scope.permits(&ApiKeyScope::RunsRead));
        assert!(!scope.permits(&ApiKeyScope::RunsWrite));
        assert!(!scope.permits(&ApiKeyScope::Admin));
    }

    #[test]
    fn has_permission_with_multiple_scopes() {
        let scopes = vec![ApiKeyScope::RunsRead, ApiKeyScope::WorkflowsRead];
        assert!(ApiKeyScope::has_permission(&scopes, &ApiKeyScope::RunsRead));
        assert!(ApiKeyScope::has_permission(
            &scopes,
            &ApiKeyScope::WorkflowsRead
        ));
        assert!(!ApiKeyScope::has_permission(
            &scopes,
            &ApiKeyScope::RunsWrite
        ));
    }

    #[test]
    fn roundtrip_display_parse() {
        let scopes = vec![
            ApiKeyScope::WorkflowsRead,
            ApiKeyScope::RunsRead,
            ApiKeyScope::RunsWrite,
            ApiKeyScope::RunsManage,
            ApiKeyScope::StatsRead,
            ApiKeyScope::Admin,
        ];
        for scope in scopes {
            let s = scope.to_string();
            let parsed: ApiKeyScope = s.parse().expect("should parse");
            assert_eq!(parsed, scope);
        }
    }

    #[test]
    fn parse_invalid_scope() {
        let result = "invalid".parse::<ApiKeyScope>();
        assert!(result.is_err());
    }

    #[test]
    fn serde_roundtrip() {
        let scope = ApiKeyScope::RunsWrite;
        let json = serde_json::to_string(&scope).expect("serialize");
        assert_eq!(json, "\"runs_write\"");
        let parsed: ApiKeyScope = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, scope);
    }

    #[test]
    fn all_non_admin_excludes_admin() {
        let scopes = ApiKeyScope::all_non_admin();
        assert!(!scopes.contains(&ApiKeyScope::Admin));
        assert_eq!(scopes.len(), 5);
    }

    #[test]
    fn member_allowed_is_read_only() {
        let allowed = ApiKeyScope::member_allowed();
        assert!(allowed.contains(&ApiKeyScope::WorkflowsRead));
        assert!(allowed.contains(&ApiKeyScope::RunsRead));
        assert!(allowed.contains(&ApiKeyScope::StatsRead));
        assert!(!allowed.contains(&ApiKeyScope::RunsWrite));
        assert!(!allowed.contains(&ApiKeyScope::RunsManage));
        assert!(!allowed.contains(&ApiKeyScope::Admin));
    }

    #[test]
    fn all_allowed_for_member_accepts_read_scopes() {
        let scopes = vec![ApiKeyScope::WorkflowsRead, ApiKeyScope::RunsRead];
        assert!(ApiKeyScope::all_allowed_for_member(&scopes));
    }

    #[test]
    fn all_allowed_for_member_rejects_write_scopes() {
        let scopes = vec![ApiKeyScope::RunsRead, ApiKeyScope::RunsWrite];
        assert!(!ApiKeyScope::all_allowed_for_member(&scopes));
    }

    #[test]
    fn all_allowed_for_member_rejects_admin() {
        let scopes = vec![ApiKeyScope::Admin];
        assert!(!ApiKeyScope::all_allowed_for_member(&scopes));
    }
}
