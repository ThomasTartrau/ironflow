//! Team operations.
//!
//! Provides CRUD and member management for Grafana teams
//! via the `/api/teams/` endpoints.

mod crud;
mod members;
pub mod types;

pub use crud::{TeamCreate, TeamDelete, TeamGet, TeamList, TeamUpdate};
pub use members::{TeamAddMember, TeamGetMembers, TeamRemoveMember};
pub use types::*;
