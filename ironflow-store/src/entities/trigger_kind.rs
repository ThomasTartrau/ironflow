//! [`TriggerKind`] — how a run was triggered.

use chrono::{DateTime, Utc};
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
        /// Schedule that created the run. `None` on runs created before it
        /// was recorded.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schedule_id: Option<Uuid>,
        /// Occurrence the run covers. `None` for a manual trigger of the
        /// schedule and on older runs.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scheduled_for: Option<DateTime<Utc>>,
    },
    /// Triggered via the REST API.
    Api,
    /// Retry of a previously failed run.
    Retry {
        /// The original run that failed.
        parent_run_id: Uuid,
    },
    /// Replay of a previously finished run on the current handler version.
    Replay {
        /// The original run that was replayed.
        original_run_id: Uuid,
    },
    /// Triggered by a parent workflow as a sub-workflow step.
    Workflow,
    /// Triggered by a message consumed from a NATS subject.
    Nats {
        /// The NATS subject the message was consumed from.
        subject: String,
    },
    /// Triggered by an internal run event (workflow chaining).
    RunEvent {
        /// The run whose event triggered this run.
        source_run_id: Uuid,
        /// The event kind that fired (e.g. `"run_failed"`).
        event_kind: String,
    },
    /// Triggered by a polling probe detecting new data.
    Polling {
        /// Name of the probe that fired (e.g. `"http"`, `"sql"`).
        probe: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

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
                schedule_id: None,
                scheduled_for: None,
            },
            TriggerKind::Cron {
                schedule: "0 */5 * * * *".to_string(),
                schedule_id: Some(Uuid::nil()),
                scheduled_for: Some(Utc.with_ymd_and_hms(2026, 10, 6, 8, 0, 0).unwrap()),
            },
            TriggerKind::Retry {
                parent_run_id: Uuid::nil(),
            },
            TriggerKind::Replay {
                original_run_id: Uuid::nil(),
            },
            TriggerKind::Nats {
                subject: "workflows.deploy".to_string(),
            },
            TriggerKind::RunEvent {
                source_run_id: Uuid::nil(),
                event_kind: "run_failed".to_string(),
            },
            TriggerKind::Polling {
                probe: "http".to_string(),
            },
        ];
        for trigger in triggers {
            let json = serde_json::to_string(&trigger).expect("serialize");
            let back: TriggerKind = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(trigger, back);
        }
    }

    #[test]
    fn nats_serializes_with_subject() {
        let trigger = TriggerKind::Nats {
            subject: "orders.created".to_string(),
        };
        let json = serde_json::to_string(&trigger).expect("serialize");
        assert!(json.contains("\"kind\":\"nats\""));
        assert!(json.contains("\"subject\":\"orders.created\""));
    }

    #[test]
    fn replay_serializes_with_original_run_id() {
        let trigger = TriggerKind::Replay {
            original_run_id: Uuid::nil(),
        };
        let json = serde_json::to_string(&trigger).expect("serialize");
        assert!(json.contains("\"kind\":\"replay\""));
        assert!(json.contains("\"original_run_id\""));
    }

    #[test]
    fn run_event_serializes_with_source() {
        let run_id = Uuid::nil();
        let trigger = TriggerKind::RunEvent {
            source_run_id: run_id,
            event_kind: "step_failed".to_string(),
        };
        let json = serde_json::to_string(&trigger).expect("serialize");
        assert!(json.contains("\"kind\":\"run_event\""));
        assert!(json.contains("\"event_kind\":\"step_failed\""));
        assert!(json.contains("\"source_run_id\""));
    }

    #[test]
    fn polling_serializes_with_probe() {
        let trigger = TriggerKind::Polling {
            probe: "http".to_string(),
        };
        let json = serde_json::to_string(&trigger).expect("serialize");
        assert!(json.contains("\"kind\":\"polling\""));
        assert!(json.contains("\"probe\":\"http\""));
    }

    #[test]
    fn cron_serializes_with_schedule_id_and_scheduled_for() {
        let trigger = TriggerKind::Cron {
            schedule: "0 * * * *".to_string(),
            schedule_id: Some(Uuid::nil()),
            scheduled_for: Some(Utc.with_ymd_and_hms(2026, 10, 6, 8, 0, 0).unwrap()),
        };
        let json = serde_json::to_string(&trigger).expect("serialize");
        assert!(json.contains("\"kind\":\"cron\""));
        assert!(json.contains("\"schedule_id\":\"00000000-0000-0000-0000-000000000000\""));
        assert!(json.contains("\"scheduled_for\":\"2026-10-06T08:00:00Z\""));

        let bare = TriggerKind::Cron {
            schedule: "0 * * * *".to_string(),
            schedule_id: None,
            scheduled_for: None,
        };
        let json = serde_json::to_string(&bare).expect("serialize");
        assert!(!json.contains("schedule_id"));
        assert!(!json.contains("scheduled_for"));
    }

    #[test]
    fn cron_without_new_fields_still_deserializes() {
        let trigger: TriggerKind =
            serde_json::from_str(r#"{"kind":"cron","schedule":"0 * * * *"}"#).expect("deserialize");
        assert_eq!(
            trigger,
            TriggerKind::Cron {
                schedule: "0 * * * *".to_string(),
                schedule_id: None,
                scheduled_for: None,
            }
        );
    }
}
