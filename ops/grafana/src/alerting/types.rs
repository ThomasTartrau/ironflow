//! Shared types for alerting operations.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// An alert rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlertRuleOutput {
    /// Rule UID.
    pub uid: Option<String>,
    /// Rule title.
    pub title: Option<String>,
    /// Rule condition.
    pub condition: Option<String>,
    /// Folder UID containing this rule.
    pub folder_uid: Option<String>,
    /// Rule group name.
    pub rule_group: Option<String>,
}

/// An alert rule group.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertRuleGroupOutput {
    /// Group name.
    pub name: Option<String>,
    /// Evaluation interval.
    pub interval: Option<String>,
    /// Rules in this group.
    pub rules: Option<Vec<AlertRuleOutput>>,
}

/// A contact point.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContactPointOutput {
    /// Contact point UID.
    pub uid: Option<String>,
    /// Contact point name.
    pub name: Option<String>,
    /// Contact point type (e.g. "email", "slack").
    #[serde(rename = "type")]
    pub type_name: Option<String>,
    /// Type-specific settings.
    pub settings: Option<Value>,
}

/// A notification policy tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationPolicyOutput {
    /// Default receiver name.
    pub receiver: Option<String>,
    /// Labels to group by.
    pub group_by: Option<Vec<String>>,
    /// Child routes.
    pub routes: Option<Vec<Value>>,
}

/// A mute timing.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MuteTimingOutput {
    /// Mute timing name.
    pub name: Option<String>,
    /// Time intervals when alerts are muted.
    pub time_intervals: Option<Vec<Value>>,
}

/// A notification template.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateOutput {
    /// Template name.
    pub name: Option<String>,
    /// Template body.
    pub template: Option<String>,
}
