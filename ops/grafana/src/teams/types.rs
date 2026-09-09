//! Shared types for team operations.

use serde::{Deserialize, Serialize};

/// Team metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamOutput {
    /// Team numeric ID.
    pub id: Option<u64>,
    /// Team name.
    pub name: Option<String>,
    /// Team email.
    pub email: Option<String>,
    /// Number of members.
    pub member_count: Option<u64>,
}

/// A team member entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamMemberOutput {
    /// User ID.
    pub user_id: Option<u64>,
    /// User login.
    pub login: Option<String>,
    /// User email.
    pub email: Option<String>,
}

/// Paginated team list response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamListOutput {
    /// Teams on this page.
    pub teams: Option<Vec<TeamOutput>>,
    /// Total count across all pages.
    pub total_count: Option<u64>,
}
