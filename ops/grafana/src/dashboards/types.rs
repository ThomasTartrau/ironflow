//! Shared types for dashboard operations.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Response from dashboard save operations (create/update).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardSaveOutput {
    /// Dashboard numeric ID.
    pub id: Option<u64>,
    /// Dashboard UID.
    pub uid: Option<String>,
    /// Dashboard URL path.
    pub url: Option<String>,
    /// Status message.
    pub status: Option<String>,
    /// Dashboard version number.
    pub version: Option<u64>,
}

/// Full dashboard model returned by get operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardGetOutput {
    /// Dashboard metadata.
    pub meta: Option<Value>,
    /// Dashboard model.
    pub dashboard: Option<Value>,
}

/// A search result entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardSearchHit {
    /// Dashboard numeric ID.
    pub id: Option<u64>,
    /// Dashboard UID.
    pub uid: Option<String>,
    /// Dashboard title.
    pub title: Option<String>,
    /// Dashboard URL.
    pub url: Option<String>,
    /// Type (dash-db, dash-folder).
    #[serde(rename = "type")]
    pub hit_type: Option<String>,
    /// Tags.
    pub tags: Option<Vec<String>>,
}

/// A dashboard version entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardVersion {
    /// Version number.
    pub id: Option<u64>,
    /// Dashboard numeric ID.
    pub dashboard_id: Option<u64>,
    /// Who created this version.
    pub created_by: Option<String>,
    /// Creation timestamp.
    pub created: Option<String>,
    /// Commit message.
    pub message: Option<String>,
}

/// A permission entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardPermission {
    /// Dashboard numeric ID.
    pub dashboard_id: Option<u64>,
    /// Role (Viewer, Editor, Admin).
    pub role: Option<String>,
    /// Permission level (1=View, 2=Edit, 4=Admin).
    pub permission: Option<u64>,
    /// Team ID.
    pub team_id: Option<u64>,
    /// User ID.
    pub user_id: Option<u64>,
}
