//! Ruler operations for managing Prometheus recording and alerting rules.
//!
//! Rules are organized by namespace and group. Mimir exposes a
//! Prometheus-compatible ruler API under `/prometheus/config/v1/rules`.

mod manage;
mod query;

pub use manage::{CreateRuleGroup, DeleteRuleGroup, DeleteRuleNamespace};
pub use query::{GetAllTenantRules, GetRuleGroup, GetRules, GetRulesByNamespace};
