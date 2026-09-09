//! Dashboard operations.
//!
//! Provides CRUD, search, versioning, and permissions for Grafana dashboards
//! via the `/api/dashboards/` endpoints.

mod crud;
mod permissions;
mod search;
pub mod types;
mod versions;

pub use crud::{DashboardCreate, DashboardDelete, DashboardGet, DashboardSave, DashboardUpdate};
pub use permissions::{DashboardGetPermissions, DashboardUpdatePermissions};
pub use search::DashboardSearch;
pub use types::*;
pub use versions::{DashboardGetVersion, DashboardGetVersions, DashboardRestoreVersion};
