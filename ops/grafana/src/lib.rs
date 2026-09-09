//! Grafana operations for Ironflow workflows, using direct HTTP calls.
//!
//! This crate provides typed Grafana API operations as Ironflow
//! [`Operation`](ironflow_core::operation::Operation) implementations. Each
//! operation sends an HTTP request to the Grafana REST API via
//! [`reqwest`] and returns a typed response.
//!
//! # Architecture
//!
//! - [`GrafanaClient`] is the central handle, wrapping a base URL and bearer token
//! - Each operation is a standalone struct implementing [`Operation`](ironflow_core::operation::Operation)
//! - All operations return `kind() == "grafana"`
//! - Parameters are set at construction time
//!
//! # Quick start
//!
//! ```no_run
//! use ironflow_ops_grafana::GrafanaClient;
//! use ironflow_ops_grafana::dashboards::DashboardGet;
//! use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
//! use std::sync::Arc;
//!
//! # async fn example() -> Result<(), ironflow_core::error::OperationError> {
//! let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
//! let grafana = GrafanaClient::from_context(&ctx).await?;
//!
//! let op = DashboardGet::new(&grafana, "my-dashboard-uid");
//! let result = op.execute(&ctx).await?;
//! # Ok(())
//! # }
//! ```
//!
//! # Tracked operations
//!
//! Every operation implements [`Operation`](ironflow_core::operation::Operation),
//! so it can be passed to `WorkflowContext::operation()` for step lifecycle
//! tracking (step record, status transitions, duration, output persistence).
//!
//! # Modules
//!
//! Operations are organized by Grafana API domain:
//!
//! | Module | Operations |
//! |--------|-----------|
//! | [`dashboards`] | Save (Create/Update), Get, Delete, Search, GetVersions, GetVersion, RestoreVersion, GetPermissions, UpdatePermissions |
//! | [`folders`] | Create, Get, Update, Delete, GetPermissions, UpdatePermissions |
//! | [`data_sources`] | Create, List, GetById, GetByUid, GetByName, Update, Delete, Query |
//! | [`annotations`] | Create, List, GetById, Update, Patch, Delete, GetTags |
//! | [`alerting`] | GetAlertRules, GetAlertRule, CreateAlertRule, UpdateAlertRule, DeleteAlertRule, GetAlertRuleGroups, UpdateAlertRuleGroup, GetContactPoints, CreateContactPoint, UpdateContactPoint, DeleteContactPoint, GetNotificationPolicies, UpdateNotificationPolicies, GetMuteTimings, CreateMuteTiming, UpdateMuteTiming, DeleteMuteTiming, GetTemplates, CreateTemplate, UpdateTemplate, DeleteTemplate |
//! | [`organizations`] | GetCurrentOrg, UpdateCurrentOrg, GetCurrentOrgUsers, AddCurrentOrgUser, UpdateCurrentOrgUser, RemoveCurrentOrgUser, List, Get, Create, Delete |
//! | [`teams`] | List, Get, Create, Update, Delete, GetMembers, AddMember, RemoveMember |
//! | [`users`] | List, GetById, Search, Update |
//! | [`service_accounts`] | List, Get, Create, Update, Delete, CreateToken, DeleteToken |
//! | [`snapshots`] | Create, List, GetByKey, DeleteByKey |
//! | [`playlists`] | List, Get, Create, Update, Delete |
//! | [`rbac`] | GetRoles, GetRole, CreateRole, UpdateRole, DeleteRole, GetRoleAssignments |
//! | [`admin`] | GetStats, SetAlertsPause, GetHealth |
//! | [`other`] | CreateShortUrl, GetFrontendSettings, RenewAuth |

pub mod admin;
pub mod alerting;
pub mod annotations;
mod client;
pub mod dashboards;
pub mod data_sources;
pub mod folders;
pub(crate) mod helpers;
pub mod organizations;
pub mod other;
pub mod playlists;
pub mod rbac;
pub mod service_accounts;
pub mod snapshots;
pub mod teams;
pub mod users;

pub use client::GrafanaClient;
