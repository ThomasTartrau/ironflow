//! [`CreatedBy`] — public representation of a run's author.

use ironflow_store::models::{Run, RunActor, TriggerKind};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Number of leading UUID characters kept in a fallback label.
const ID_PREFIX_LEN: usize = 8;

/// What kind of principal triggered a run.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CreatedByKind {
    /// A user authenticated via JWT.
    User,
    /// An API key.
    ApiKey,
    /// No authenticated caller: cron, webhook, or a programmatic trigger.
    System,
}

/// The author of a run, as exposed by the API.
///
/// Always present on a [`RunResponse`](crate::entities::RunResponse): a run with
/// no recorded author (including every run created before authorship tracking)
/// reports [`CreatedByKind::System`] with a label derived from its trigger, so
/// clients never have to handle a missing value.
///
/// # Examples
///
/// ```
/// use ironflow_api::entities::{CreatedBy, CreatedByKind};
///
/// // Built from a Run via `CreatedBy::from(&run)`.
/// let system = CreatedBy {
///     kind: CreatedByKind::System,
///     id: None,
///     label: "manual".to_string(),
/// };
/// assert_eq!(system.id, None);
/// ```
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreatedBy {
    /// What kind of principal triggered the run.
    pub kind: CreatedByKind,
    /// Identifier of the principal: the user ID, the API key ID, or `None` for
    /// [`CreatedByKind::System`].
    pub id: Option<Uuid>,
    /// Human-readable label. Never empty.
    ///
    /// The username for a user, `"key-name (username)"` for an API key, and the
    /// trigger description for a system run.
    pub label: String,
}

/// Shorten a UUID to its leading characters for a fallback label.
fn short_id(id: Uuid) -> String {
    let text = id.to_string();
    text[..ID_PREFIX_LEN.min(text.len())].to_string()
}

/// Describe an unauthenticated trigger.
fn trigger_label(trigger: &TriggerKind) -> String {
    match trigger {
        TriggerKind::Manual => "manual".to_string(),
        TriggerKind::Api => "api".to_string(),
        TriggerKind::Workflow => "workflow".to_string(),
        TriggerKind::Retry { .. } => "retry".to_string(),
        TriggerKind::Webhook { path } => path.clone(),
        TriggerKind::Cron { schedule } => schedule.clone(),
        TriggerKind::Nats { subject } => format!("nats:{subject}"),
        TriggerKind::RunEvent { event_kind, .. } => format!("event:{event_kind}"),
        TriggerKind::Polling { probe } => format!("polling:{probe}"),
    }
}

impl From<&Run> for CreatedBy {
    fn from(run: &Run) -> Self {
        match run.created_by {
            None => CreatedBy {
                kind: CreatedByKind::System,
                id: None,
                label: trigger_label(&run.trigger),
            },
            Some(RunActor::User { user_id }) => CreatedBy {
                kind: CreatedByKind::User,
                id: Some(user_id),
                label: run
                    .created_by_label
                    .clone()
                    .unwrap_or_else(|| format!("user {}", short_id(user_id))),
            },
            Some(RunActor::ApiKey { api_key_id, .. }) => CreatedBy {
                kind: CreatedByKind::ApiKey,
                id: Some(api_key_id),
                label: run
                    .created_by_label
                    .clone()
                    .unwrap_or_else(|| format!("key {}", short_id(api_key_id))),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use ironflow_store::api_key_store::ApiKeyStore;
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::models::{ApiKeyScope, NewApiKey, NewRun, NewUser};
    use ironflow_store::store::RunStore;
    use ironflow_store::user_store::UserStore;
    use serde_json::json;

    use super::*;

    /// Build a run through the real store, so `created_by_label` is resolved the
    /// same way the API sees it at runtime.
    async fn run_with(
        store: &InMemoryStore,
        trigger: TriggerKind,
        created_by: Option<RunActor>,
    ) -> Run {
        store
            .create_run(NewRun {
                workflow_name: "deploy".to_string(),
                trigger,
                payload: json!({}),
                max_retries: 0,
                handler_version: None,
                labels: HashMap::new(),
                scheduled_at: None,
                created_by,
                idempotency_key: None,
                max_cost_usd: None,
            })
            .await
            .expect("create run")
            .into_run()
    }

    async fn seed_user(store: &InMemoryStore, username: &str) -> Uuid {
        store
            .create_user(NewUser {
                email: format!("{username}@example.com"),
                username: username.to_string(),
                password_hash: "hash".to_string(),
                is_admin: Some(false),
            })
            .await
            .expect("create user")
            .id
    }

    async fn seed_api_key(store: &InMemoryStore, user_id: Uuid, name: &str) -> Uuid {
        store
            .create_api_key(NewApiKey {
                user_id,
                name: name.to_string(),
                key_hash: "hash".to_string(),
                key_prefix: "irfl_0000".to_string(),
                scopes: vec![ApiKeyScope::RunsWrite],
                expires_at: None,
                rate_limit_override: None,
            })
            .await
            .expect("create api key")
            .id
    }

    #[tokio::test]
    async fn user_actor_uses_the_resolved_username() {
        let store = InMemoryStore::new();
        let user_id = seed_user(&store, "alice").await;
        let run = run_with(&store, TriggerKind::Api, Some(RunActor::User { user_id })).await;

        let created_by = CreatedBy::from(&run);
        assert_eq!(created_by.kind, CreatedByKind::User);
        assert_eq!(created_by.id, Some(user_id));
        assert_eq!(created_by.label, "alice");
    }

    #[tokio::test]
    async fn user_actor_falls_back_to_a_short_id() {
        let store = InMemoryStore::new();
        let user_id = Uuid::now_v7();
        let run = run_with(&store, TriggerKind::Api, Some(RunActor::User { user_id })).await;

        let created_by = CreatedBy::from(&run);
        assert_eq!(created_by.kind, CreatedByKind::User);
        assert_eq!(created_by.id, Some(user_id));
        assert_eq!(created_by.label, format!("user {}", short_id(user_id)));
    }

    #[tokio::test]
    async fn api_key_actor_exposes_the_key_id_and_a_combined_label() {
        let store = InMemoryStore::new();
        let user_id = seed_user(&store, "alice").await;
        let api_key_id = seed_api_key(&store, user_id, "ci-deploy").await;
        let run = run_with(
            &store,
            TriggerKind::Api,
            Some(RunActor::ApiKey {
                api_key_id,
                user_id,
            }),
        )
        .await;

        let created_by = CreatedBy::from(&run);
        assert_eq!(created_by.kind, CreatedByKind::ApiKey);
        assert_eq!(created_by.id, Some(api_key_id));
        assert_eq!(created_by.label, "ci-deploy (alice)");
    }

    #[tokio::test]
    async fn api_key_actor_falls_back_to_a_short_id() {
        let store = InMemoryStore::new();
        let api_key_id = Uuid::now_v7();
        let run = run_with(
            &store,
            TriggerKind::Api,
            Some(RunActor::ApiKey {
                api_key_id,
                user_id: Uuid::now_v7(),
            }),
        )
        .await;

        let created_by = CreatedBy::from(&run);
        assert_eq!(created_by.kind, CreatedByKind::ApiKey);
        assert_eq!(created_by.label, format!("key {}", short_id(api_key_id)));
    }

    #[tokio::test]
    async fn system_label_is_derived_from_every_trigger() {
        let store = InMemoryStore::new();
        let cases = [
            (TriggerKind::Manual, "manual"),
            (TriggerKind::Api, "api"),
            (TriggerKind::Workflow, "workflow"),
            (
                TriggerKind::Retry {
                    parent_run_id: Uuid::now_v7(),
                },
                "retry",
            ),
            (
                TriggerKind::Webhook {
                    path: "/hooks/github".to_string(),
                },
                "/hooks/github",
            ),
            (
                TriggerKind::Cron {
                    schedule: "0 */5 * * * *".to_string(),
                },
                "0 */5 * * * *",
            ),
            (
                TriggerKind::Nats {
                    subject: "orders.created".to_string(),
                },
                "nats:orders.created",
            ),
            (
                TriggerKind::RunEvent {
                    source_run_id: Uuid::now_v7(),
                    event_kind: "completed".to_string(),
                },
                "event:completed",
            ),
            (
                TriggerKind::Polling {
                    probe: "http".to_string(),
                },
                "polling:http",
            ),
        ];

        for (trigger, expected) in cases {
            let run = run_with(&store, trigger, None).await;
            let created_by = CreatedBy::from(&run);

            assert_eq!(created_by.kind, CreatedByKind::System);
            assert_eq!(created_by.id, None);
            assert_eq!(created_by.label, expected);
        }
    }

    #[tokio::test]
    async fn a_pre_migration_run_reports_a_system_author() {
        let store = InMemoryStore::new();
        // A run predating authorship tracking has both columns NULL.
        let run = run_with(&store, TriggerKind::Manual, None).await;

        let created_by = CreatedBy::from(&run);
        assert_eq!(created_by.kind, CreatedByKind::System);
        assert_eq!(created_by.id, None);
        assert_eq!(created_by.label, "manual");
    }

    #[tokio::test]
    async fn label_is_never_empty() {
        let store = InMemoryStore::new();
        let run = run_with(
            &store,
            TriggerKind::Webhook {
                path: "/h".to_string(),
            },
            None,
        )
        .await;

        assert!(!CreatedBy::from(&run).label.is_empty());
    }

    #[test]
    fn short_id_keeps_eight_characters() {
        let id = Uuid::now_v7();
        assert_eq!(short_id(id).len(), ID_PREFIX_LEN);
        assert!(id.to_string().starts_with(&short_id(id)));
    }

    #[test]
    fn serde_uses_snake_case_kinds() {
        let created_by = CreatedBy {
            kind: CreatedByKind::ApiKey,
            id: Some(Uuid::now_v7()),
            label: "ci (alice)".to_string(),
        };

        let json = serde_json::to_value(&created_by).expect("serialize");
        assert_eq!(json["kind"], "api_key");

        let back: CreatedBy = serde_json::from_value(json).expect("deserialize");
        assert_eq!(back, created_by);
    }
}
