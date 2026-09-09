//! Alerting operations.
//!
//! Provides alert rules, rule groups, contact points, notification policies,
//! mute timings, and notification templates via the Grafana provisioning API
//! (`/api/v1/provisioning/`).

mod contact_points;
mod mute_timings;
mod notification_policies;
mod rule_groups;
mod rules;
mod templates;
pub mod types;

pub use contact_points::{
    ContactPointCreate, ContactPointDelete, ContactPointList, ContactPointUpdate,
};
pub use mute_timings::{MuteTimingCreate, MuteTimingDelete, MuteTimingList, MuteTimingUpdate};
pub use notification_policies::{NotificationPolicyGet, NotificationPolicyUpdate};
pub use rule_groups::{AlertRuleGroupGet, AlertRuleGroupUpdate};
pub use rules::{AlertRuleCreate, AlertRuleDelete, AlertRuleGet, AlertRuleList, AlertRuleUpdate};
pub use templates::{TemplateCreate, TemplateDelete, TemplateList, TemplateUpdate};
pub use types::*;
