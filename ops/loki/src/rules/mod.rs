//! Alerting and recording rule management operations.
//!
//! These operations manage Loki's ruler component, which evaluates LogQL
//! expressions on a schedule and fires alerts or records metrics.
//!
//! Operations are split into two sub-modules:
//!
//! - `query` -- read-only operations: list rules, get a group, list alerts
//! - `manage` -- write operations: create, delete rule groups and namespaces

mod manage;
mod query;

pub use manage::{CreateRuleGroup, DeleteRuleGroup, DeleteRuleNamespace};
pub use query::{GetAlerts, GetRuleGroup, GetRules, GetRulesByNamespace};
