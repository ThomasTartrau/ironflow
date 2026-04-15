//! [`TriggerKind`] — how a run was triggered.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// How a run was triggered.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::TriggerKind;
///
/// let trigger = TriggerKind::Manual;
/// let json = serde_json::to_string(&trigger).unwrap();
/// assert!(json.contains("manual"));
/// ```
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum TriggerKind {
    /// Triggered manually (CLI or programmatic call).
    Manual,
    /// Triggered by an incoming webhook.
    Webhook {
        /// The webhook path that received the request.
        path: String,
    },
    /// Triggered by a cron schedule.
    Cron {
        /// The cron expression that fired.
        schedule: String,
    },
    /// Triggered via the REST API.
    Api,
    /// Retry of a previously failed run.
    Retry {
        /// The original run that failed.
        parent_run_id: Uuid,
    },
    /// Triggered by a parent workflow as a sub-workflow step.
    Workflow,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_roundtrip() {
        let triggers = vec![
            TriggerKind::Manual,
            TriggerKind::Api,
            TriggerKind::Webhook {
                path: "/hooks/gh".to_string(),
            },
            TriggerKind::Cron {
                schedule: "0 */5 * * * *".to_string(),
            },
            TriggerKind::Retry {
                parent_run_id: Uuid::nil(),
            },
        ];
        for trigger in triggers {
            let json = serde_json::to_string(&trigger).expect("serialize");
            let back: TriggerKind = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(trigger, back);
        }
    }
}
