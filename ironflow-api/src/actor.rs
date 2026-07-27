//! Mapping from an authenticated caller to a persisted run author.

use ironflow_auth::extractor::{AuthMethod, Authenticated};
use ironflow_store::models::RunActor;

/// Build the [`RunActor`] to persist for a run created by `auth`.
///
/// An API key run records both the key and its owner, so filtering runs by user
/// also matches the runs triggered by that user's keys.
///
/// # Examples
///
/// ```no_run
/// use ironflow_api::actor::run_actor_of;
/// use ironflow_auth::extractor::Authenticated;
///
/// fn example(auth: &Authenticated) {
///     let actor = run_actor_of(auth);
///     println!("{:?}", actor);
/// }
/// ```
pub fn run_actor_of(auth: &Authenticated) -> RunActor {
    match auth.method {
        AuthMethod::Jwt { .. } => RunActor::User {
            user_id: auth.user_id,
        },
        AuthMethod::ApiKey { key_id, .. } => RunActor::ApiKey {
            api_key_id: key_id,
            user_id: auth.user_id,
        },
    }
}

#[cfg(test)]
mod tests {
    use ironflow_store::entities::ApiKeyScope;
    use uuid::Uuid;

    use super::*;

    #[test]
    fn jwt_caller_maps_to_a_user_actor() {
        let user_id = Uuid::now_v7();
        let auth = Authenticated {
            user_id,
            method: AuthMethod::Jwt {
                username: "alice".to_string(),
                is_admin: true,
            },
        };

        assert_eq!(run_actor_of(&auth), RunActor::User { user_id });
    }

    #[test]
    fn api_key_caller_maps_to_an_api_key_actor_carrying_its_owner() {
        let user_id = Uuid::now_v7();
        let key_id = Uuid::now_v7();
        let auth = Authenticated {
            user_id,
            method: AuthMethod::ApiKey {
                key_id,
                key_name: "ci-deploy".to_string(),
                scopes: vec![ApiKeyScope::RunsWrite],
                owner_is_admin: true,
            },
        };

        assert_eq!(
            run_actor_of(&auth),
            RunActor::ApiKey {
                api_key_id: key_id,
                user_id,
            }
        );
    }
}
