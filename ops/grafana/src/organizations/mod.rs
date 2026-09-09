//! Organization operations.
//!
//! Provides current-org management and server-admin org CRUD for Grafana
//! via the `/api/org/` and `/api/orgs/` endpoints.

mod admin;
mod current;
pub mod types;

pub use admin::{OrgCreate, OrgDelete, OrgGet, OrgList};
pub use current::{
    OrgAddCurrentUser, OrgGetCurrent, OrgGetCurrentUsers, OrgRemoveCurrentUser, OrgUpdateCurrent,
    OrgUpdateCurrentUser,
};
pub use types::*;
