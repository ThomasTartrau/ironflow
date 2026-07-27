//! [`RunActor`] — the authenticated principal that created a run.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The authenticated principal that created a run.
///
/// `None` on a [`Run`](crate::entities::Run) means the run was not triggered by
/// an authenticated caller: cron, webhook, or a programmatic in-process call.
///
/// [`RunActor::ApiKey`] carries both the key and its owner, so filtering runs by
/// user also matches the runs triggered by that user's API keys.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::RunActor;
/// use uuid::Uuid;
///
/// let actor = RunActor::User { user_id: Uuid::now_v7() };
/// let json = serde_json::to_string(&actor)?;
/// assert!(json.contains("user"));
/// # Ok::<(), serde_json::Error>(())
/// ```
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum RunActor {
    /// A user authenticated via JWT (cookie or `Authorization: Bearer`).
    User {
        /// The user that triggered the run.
        user_id: Uuid,
    },
    /// An API key, plus the user that owns it.
    ApiKey {
        /// The API key that triggered the run.
        api_key_id: Uuid,
        /// The owner of the API key.
        user_id: Uuid,
    },
}

impl RunActor {
    /// The user this actor is attributed to.
    ///
    /// For [`RunActor::ApiKey`], this is the key owner.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_store::entities::RunActor;
    /// use uuid::Uuid;
    ///
    /// let owner = Uuid::now_v7();
    /// let actor = RunActor::ApiKey { api_key_id: Uuid::now_v7(), user_id: owner };
    /// assert_eq!(actor.user_id(), owner);
    /// ```
    pub fn user_id(&self) -> Uuid {
        match self {
            RunActor::User { user_id } | RunActor::ApiKey { user_id, .. } => *user_id,
        }
    }

    /// The API key that triggered the run, if any.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_store::entities::RunActor;
    /// use uuid::Uuid;
    ///
    /// let actor = RunActor::User { user_id: Uuid::now_v7() };
    /// assert!(actor.api_key_id().is_none());
    /// ```
    pub fn api_key_id(&self) -> Option<Uuid> {
        match self {
            RunActor::User { .. } => None,
            RunActor::ApiKey { api_key_id, .. } => Some(*api_key_id),
        }
    }

    /// Rebuild an actor from the two nullable columns persisted on a run.
    ///
    /// Returns `None` when no user is recorded — an API key id without a user
    /// is not a representable state and is discarded.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_store::entities::RunActor;
    /// use uuid::Uuid;
    ///
    /// assert!(RunActor::from_columns(None, None).is_none());
    ///
    /// let user_id = Uuid::now_v7();
    /// assert_eq!(
    ///     RunActor::from_columns(Some(user_id), None),
    ///     Some(RunActor::User { user_id })
    /// );
    /// ```
    pub fn from_columns(user_id: Option<Uuid>, api_key_id: Option<Uuid>) -> Option<Self> {
        match (user_id, api_key_id) {
            (Some(user_id), Some(api_key_id)) => Some(RunActor::ApiKey {
                api_key_id,
                user_id,
            }),
            (Some(user_id), None) => Some(RunActor::User { user_id }),
            (None, _) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_roundtrip() {
        let actors = vec![
            RunActor::User {
                user_id: Uuid::now_v7(),
            },
            RunActor::ApiKey {
                api_key_id: Uuid::now_v7(),
                user_id: Uuid::now_v7(),
            },
        ];

        for actor in actors {
            let json = serde_json::to_string(&actor).expect("serialize");
            let back: RunActor = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(actor, back);
        }
    }

    #[test]
    fn user_variant_is_tagged_user() {
        let actor = RunActor::User {
            user_id: Uuid::now_v7(),
        };
        let json = serde_json::to_value(&actor).expect("serialize");
        assert_eq!(json["kind"], "user");
    }

    #[test]
    fn api_key_variant_is_tagged_api_key() {
        let actor = RunActor::ApiKey {
            api_key_id: Uuid::now_v7(),
            user_id: Uuid::now_v7(),
        };
        let json = serde_json::to_value(&actor).expect("serialize");
        assert_eq!(json["kind"], "api_key");
    }

    #[test]
    fn user_id_returns_owner_for_both_variants() {
        let user_id = Uuid::now_v7();
        assert_eq!(RunActor::User { user_id }.user_id(), user_id);
        assert_eq!(
            RunActor::ApiKey {
                api_key_id: Uuid::now_v7(),
                user_id,
            }
            .user_id(),
            user_id
        );
    }

    #[test]
    fn api_key_id_is_none_for_user_variant() {
        let actor = RunActor::User {
            user_id: Uuid::now_v7(),
        };
        assert!(actor.api_key_id().is_none());
    }

    #[test]
    fn api_key_id_is_some_for_api_key_variant() {
        let api_key_id = Uuid::now_v7();
        let actor = RunActor::ApiKey {
            api_key_id,
            user_id: Uuid::now_v7(),
        };
        assert_eq!(actor.api_key_id(), Some(api_key_id));
    }

    #[test]
    fn from_columns_both_none() {
        assert!(RunActor::from_columns(None, None).is_none());
    }

    #[test]
    fn from_columns_user_only() {
        let user_id = Uuid::now_v7();
        assert_eq!(
            RunActor::from_columns(Some(user_id), None),
            Some(RunActor::User { user_id })
        );
    }

    #[test]
    fn from_columns_user_and_key() {
        let user_id = Uuid::now_v7();
        let api_key_id = Uuid::now_v7();
        assert_eq!(
            RunActor::from_columns(Some(user_id), Some(api_key_id)),
            Some(RunActor::ApiKey {
                api_key_id,
                user_id
            })
        );
    }

    #[test]
    fn from_columns_key_without_user_is_discarded() {
        assert!(RunActor::from_columns(None, Some(Uuid::now_v7())).is_none());
    }
}
