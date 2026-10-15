//! Per-route authorization of an authenticated caller.
//!
//! Every user-facing handler starts with [`AuthorizeExt::require`]: an API key
//! must hold the route's [`ApiKeyScope`] (or `admin`), and admin-only routes
//! also need an admin user behind the session or the key. JWT sessions are
//! not scope-checked, only the admin flag applies to them.

use ironflow_auth::extractor::{AuthMethod, Authenticated};
use ironflow_store::entities::ApiKeyScope;

use crate::error::ApiError;

/// Scope and admin checks on an [`Authenticated`] caller.
///
/// Lives in `ironflow-api` rather than `ironflow-auth` because it returns
/// [`ApiError`], which `ironflow-auth` cannot depend on.
///
/// # Examples
///
/// ```
/// use ironflow_api::authorization::AuthorizeExt;
/// use ironflow_api::error::ApiError;
/// use ironflow_auth::extractor::{AuthMethod, Authenticated};
/// use ironflow_store::entities::ApiKeyScope;
/// use uuid::Uuid;
///
/// # fn example() -> Result<(), ApiError> {
/// let auth = Authenticated {
///     user_id: Uuid::now_v7(),
///     method: AuthMethod::ApiKey {
///         key_id: Uuid::now_v7(),
///         key_name: "ci".to_string(),
///         scopes: vec![ApiKeyScope::RunsRead],
///         owner_is_admin: false,
///     },
/// };
/// auth.require(ApiKeyScope::RunsRead, false)?;
/// # Ok(())
/// # }
/// ```
pub trait AuthorizeExt {
    /// Check that the caller may use a route needing `scope`.
    ///
    /// An API key must hold `scope` (the `admin` scope implies every scope).
    /// When `admin_only` is set, the caller must also be an admin: a JWT
    /// session must belong to an admin, and an API key must be owned by an
    /// admin (read at request time, so a demotion takes effect immediately).
    /// The scope is checked before the admin flag.
    ///
    /// # Errors
    ///
    /// - [`ApiError::InsufficientScope`] if an API key lacks `scope`
    /// - [`ApiError::Forbidden`] if `admin_only` is set and the caller (or the
    ///   key owner) is not an admin
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_api::authorization::AuthorizeExt;
    /// use ironflow_api::error::ApiError;
    /// use ironflow_auth::extractor::{AuthMethod, Authenticated};
    /// use ironflow_store::entities::ApiKeyScope;
    /// use uuid::Uuid;
    ///
    /// # fn example() -> Result<(), ApiError> {
    /// let auth = Authenticated {
    ///     user_id: Uuid::now_v7(),
    ///     method: AuthMethod::Jwt {
    ///         username: "alice".to_string(),
    ///         is_admin: true,
    ///     },
    /// };
    /// auth.require(ApiKeyScope::Admin, true)?;
    /// # Ok(())
    /// # }
    /// ```
    fn require(&self, scope: ApiKeyScope, admin_only: bool) -> Result<(), ApiError>;
}

impl AuthorizeExt for Authenticated {
    fn require(&self, scope: ApiKeyScope, admin_only: bool) -> Result<(), ApiError> {
        match &self.method {
            AuthMethod::Jwt { is_admin, .. } => {
                if admin_only && !is_admin {
                    return Err(ApiError::Forbidden);
                }
            }
            AuthMethod::ApiKey {
                scopes,
                owner_is_admin,
                ..
            } => {
                if !ApiKeyScope::has_permission(scopes, &scope) {
                    return Err(ApiError::InsufficientScope);
                }
                if admin_only && !owner_is_admin {
                    return Err(ApiError::Forbidden);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;

    fn jwt(is_admin: bool) -> Authenticated {
        Authenticated {
            user_id: Uuid::now_v7(),
            method: AuthMethod::Jwt {
                username: "user".to_string(),
                is_admin,
            },
        }
    }

    fn key(scopes: Vec<ApiKeyScope>, owner_is_admin: bool) -> Authenticated {
        Authenticated {
            user_id: Uuid::now_v7(),
            method: AuthMethod::ApiKey {
                key_id: Uuid::now_v7(),
                key_name: "key".to_string(),
                scopes,
                owner_is_admin,
            },
        }
    }

    #[test]
    fn scope_jwt_member_on_admin_only_route_is_forbidden() {
        let result = jwt(false).require(ApiKeyScope::RunsManage, true);
        assert!(matches!(result, Err(ApiError::Forbidden)));
    }

    #[test]
    fn scope_jwt_member_on_regular_route_passes() {
        assert!(jwt(false).require(ApiKeyScope::RunsRead, false).is_ok());
    }

    #[test]
    fn scope_jwt_admin_passes_admin_only_route() {
        assert!(jwt(true).require(ApiKeyScope::Admin, true).is_ok());
    }

    #[test]
    fn scope_key_with_another_scope_is_insufficient() {
        let auth = key(vec![ApiKeyScope::StatsRead], true);
        let result = auth.require(ApiKeyScope::RunsRead, false);
        assert!(matches!(result, Err(ApiError::InsufficientScope)));
    }

    #[test]
    fn scope_key_with_the_exact_scope_passes() {
        let auth = key(vec![ApiKeyScope::RunsRead], false);
        assert!(auth.require(ApiKeyScope::RunsRead, false).is_ok());
    }

    #[test]
    fn scope_admin_key_satisfies_any_scope() {
        let auth = key(vec![ApiKeyScope::Admin], true);
        for scope in ApiKeyScope::all_non_admin() {
            assert!(auth.require(scope, false).is_ok());
        }
        assert!(auth.require(ApiKeyScope::Admin, true).is_ok());
    }

    #[test]
    fn scope_key_of_member_on_admin_only_route_is_forbidden() {
        let auth = key(vec![ApiKeyScope::RunsManage], false);
        let result = auth.require(ApiKeyScope::RunsManage, true);
        assert!(matches!(result, Err(ApiError::Forbidden)));
    }

    #[test]
    fn scope_is_checked_before_the_owner_admin_flag() {
        let auth = key(vec![ApiKeyScope::StatsRead], false);
        let result = auth.require(ApiKeyScope::RunsManage, true);
        assert!(matches!(result, Err(ApiError::InsufficientScope)));
    }

    #[test]
    fn scope_admin_key_of_member_is_forbidden_on_admin_only_route() {
        let auth = key(vec![ApiKeyScope::Admin], false);
        let result = auth.require(ApiKeyScope::Admin, true);
        assert!(matches!(result, Err(ApiError::Forbidden)));
    }
}
