//! Shared types for organization operations.

use serde::{Deserialize, Serialize};

/// Organization metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrgOutput {
    /// Organization numeric ID.
    pub id: Option<u64>,
    /// Organization name.
    pub name: Option<String>,
}

/// An organization user entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrgUserOutput {
    /// Organization ID.
    pub org_id: Option<u64>,
    /// User ID.
    pub user_id: Option<u64>,
    /// User login.
    pub login: Option<String>,
    /// User role in the org.
    pub role: Option<String>,
    /// User email.
    pub email: Option<String>,
}
