//! Axum extractors for authenticated callers.
//!
//! Provides three extractors:
//!
//! - [`AuthenticatedUser`] -- JWT only (cookie or `Authorization: Bearer` header)
//! - [`ApiKeyAuth`] -- API key only (`irfl_...` prefix)
//! - [`Authenticated`] -- Dual auth: API key OR JWT

use std::sync::Arc;

use axum::Json;
use axum::extract::{FromRef, FromRequestParts};
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use axum_extra::extract::cookie::CookieJar;
use chrono::Utc;
use ironflow_store::entities::ApiKeyScope;
use ironflow_store::store::Store;
use serde_json::json;
use uuid::Uuid;

use crate::cookies::AUTH_COOKIE_NAME;
use crate::jwt::{AccessToken, JwtConfig};
use crate::password;

// ---------------------------------------------------------------------------
// AuthenticatedUser (JWT only)
// ---------------------------------------------------------------------------

/// An authenticated user extracted from a JWT.
///
/// Use as an Axum handler parameter to enforce JWT authentication.
/// Requires `Arc<JwtConfig>` to be extractable from state via `FromRef`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_auth::extractor::AuthenticatedUser;
///
/// async fn protected(user: AuthenticatedUser) -> String {
///     format!("Hello, {}!", user.username)
/// }
/// ```
#[derive(Debug, Clone)]
pub struct AuthenticatedUser {
    /// The user's unique identifier.
    pub user_id: Uuid,
    /// The user's username.
    pub username: String,
    /// Whether the user is an administrator.
    pub is_admin: bool,
}

impl<S> FromRequestParts<S> for AuthenticatedUser
where
    S: Send + Sync,
    Arc<JwtConfig>: FromRef<S>,
{
    type Rejection = AuthRejection;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let jwt_config = Arc::<JwtConfig>::from_ref(state);

        let jar = CookieJar::from_headers(&parts.headers);
        let token = jar
            .get(AUTH_COOKIE_NAME)
            .map(|c| c.value().to_string())
            .or_else(|| {
                parts
                    .headers
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.strip_prefix("Bearer "))
                    .map(|t| t.to_string())
            });

        let token = token.ok_or(AuthRejection {
            status: StatusCode::UNAUTHORIZED,
            code: "MISSING_TOKEN",
            message: "No authentication token provided",
        })?;

        let claims = AccessToken::decode(&token, &jwt_config).map_err(|_| AuthRejection {
            status: StatusCode::UNAUTHORIZED,
            code: "INVALID_TOKEN",
            message: "Invalid or expired authentication token",
        })?;

        Ok(AuthenticatedUser {
            user_id: claims.user_id,
            username: claims.username,
            is_admin: claims.is_admin,
        })
    }
}

/// Rejection type when JWT authentication fails.
pub struct AuthRejection {
    status: StatusCode,
    code: &'static str,
    message: &'static str,
}

impl IntoResponse for AuthRejection {
    fn into_response(self) -> Response {
        let body = json!({
            "error": {
                "code": self.code,
                "message": self.message,
            }
        });
        (self.status, Json(body)).into_response()
    }
}

// ---------------------------------------------------------------------------
// ApiKeyAuth (API key only)
// ---------------------------------------------------------------------------

/// API key prefix used to distinguish API keys from JWT tokens.
pub const API_KEY_PREFIX: &str = "irfl_";

/// Number of hex characters kept after [`API_KEY_PREFIX`] to form the stored prefix.
pub const API_KEY_SUFFIX_LEN: usize = 8;

/// An authenticated caller via API key.
///
/// Use as an Axum handler parameter to enforce API key authentication.
///
/// # Examples
///
/// ```no_run
/// use ironflow_auth::extractor::ApiKeyAuth;
///
/// async fn protected(key: ApiKeyAuth) -> String {
///     format!("Key {} (user {})", key.key_name, key.user_id)
/// }
/// ```
#[derive(Debug, Clone)]
pub struct ApiKeyAuth {
    /// The API key ID.
    pub key_id: Uuid,
    /// The owner user ID.
    pub user_id: Uuid,
    /// The API key name.
    pub key_name: String,
    /// Scopes granted to this key.
    pub scopes: Vec<ApiKeyScope>,
    /// Whether the key owner is an admin (checked at request time).
    pub owner_is_admin: bool,
}

impl ApiKeyAuth {
    /// Check if the API key has a specific scope.
    pub fn has_scope(&self, required: &ApiKeyScope) -> bool {
        ApiKeyScope::has_permission(&self.scopes, required)
    }
}

/// Rejection type when API key authentication fails.
pub struct ApiKeyRejection {
    status: StatusCode,
    code: &'static str,
    message: &'static str,
}

impl IntoResponse for ApiKeyRejection {
    fn into_response(self) -> Response {
        let body = json!({
            "error": {
                "code": self.code,
                "message": self.message,
            }
        });
        (self.status, Json(body)).into_response()
    }
}

impl<S> FromRequestParts<S> for ApiKeyAuth
where
    S: Send + Sync,
    Arc<dyn Store>: FromRef<S>,
{
    type Rejection = ApiKeyRejection;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let store = Arc::<dyn Store>::from_ref(state);

        let token = parts
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .ok_or(ApiKeyRejection {
                status: StatusCode::UNAUTHORIZED,
                code: "MISSING_TOKEN",
                message: "No authentication token provided",
            })?;

        if !token.starts_with(API_KEY_PREFIX) {
            return Err(ApiKeyRejection {
                status: StatusCode::UNAUTHORIZED,
                code: "INVALID_TOKEN",
                message: "Expected API key (irfl_...) in Authorization header",
            });
        }

        let suffix_len = (token.len() - API_KEY_PREFIX.len()).min(API_KEY_SUFFIX_LEN);
        let prefix = &token[..API_KEY_PREFIX.len() + suffix_len];

        let api_key = store
            .find_api_key_by_prefix(prefix)
            .await
            .map_err(|_| ApiKeyRejection {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                code: "INTERNAL_ERROR",
                message: "Failed to look up API key",
            })?
            .ok_or(ApiKeyRejection {
                status: StatusCode::UNAUTHORIZED,
                code: "INVALID_TOKEN",
                message: "Invalid API key",
            })?;

        if !api_key.is_active {
            return Err(ApiKeyRejection {
                status: StatusCode::UNAUTHORIZED,
                code: "KEY_DISABLED",
                message: "API key is disabled",
            });
        }

        if let Some(expires_at) = api_key.expires_at
            && expires_at < Utc::now()
        {
            return Err(ApiKeyRejection {
                status: StatusCode::UNAUTHORIZED,
                code: "KEY_EXPIRED",
                message: "API key has expired",
            });
        }

        let valid = password::verify(token, &api_key.key_hash).map_err(|_| ApiKeyRejection {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "INTERNAL_ERROR",
            message: "Failed to verify API key",
        })?;

        if !valid {
            return Err(ApiKeyRejection {
                status: StatusCode::UNAUTHORIZED,
                code: "INVALID_TOKEN",
                message: "Invalid API key",
            });
        }

        let _ = store.touch_api_key(api_key.id).await;

        let owner = store
            .find_user_by_id(api_key.user_id)
            .await
            .map_err(|_| ApiKeyRejection {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                code: "INTERNAL_ERROR",
                message: "Failed to look up API key owner",
            })?;
        let owner_is_admin = owner.map(|u| u.is_admin).unwrap_or(false);

        Ok(ApiKeyAuth {
            key_id: api_key.id,
            user_id: api_key.user_id,
            key_name: api_key.name,
            scopes: api_key.scopes,
            owner_is_admin,
        })
    }
}

// ---------------------------------------------------------------------------
// Authenticated (dual: API key OR JWT)
// ---------------------------------------------------------------------------

/// An authenticated caller, either via API key or JWT.
///
/// If the `Authorization: Bearer` token starts with `irfl_`, API key auth is used.
/// Otherwise, JWT auth is attempted (cookie first, then header).
///
/// # Examples
///
/// ```no_run
/// use ironflow_auth::extractor::Authenticated;
///
/// async fn protected(auth: Authenticated) -> String {
///     format!("User {}", auth.user_id)
/// }
/// ```
#[derive(Debug, Clone)]
pub struct Authenticated {
    /// The authenticated user's ID.
    pub user_id: Uuid,
    /// The authentication method used.
    pub method: AuthMethod,
}

/// How the caller was authenticated.
#[derive(Debug, Clone)]
pub enum AuthMethod {
    /// Authenticated via JWT (cookie or Bearer header).
    Jwt {
        /// The user's username.
        username: String,
        /// Whether the user is an admin.
        is_admin: bool,
    },
    /// Authenticated via API key.
    ApiKey {
        /// The API key ID.
        key_id: Uuid,
        /// The API key name.
        key_name: String,
        /// Scopes granted to this key.
        scopes: Vec<ApiKeyScope>,
        /// Whether the key owner is an admin (checked at request time).
        owner_is_admin: bool,
    },
}

impl Authenticated {
    /// Whether the authenticated caller has admin privileges.
    ///
    /// For JWT users, checks the `is_admin` claim.
    /// For API key users, checks the owner's current admin status
    /// (fetched at request time, so demotions take effect immediately).
    pub fn is_admin(&self) -> bool {
        match &self.method {
            AuthMethod::Jwt { is_admin, .. } => *is_admin,
            AuthMethod::ApiKey { owner_is_admin, .. } => *owner_is_admin,
        }
    }
}

impl<S> FromRequestParts<S> for Authenticated
where
    S: Send + Sync,
    Arc<JwtConfig>: FromRef<S>,
    Arc<dyn Store>: FromRef<S>,
{
    type Rejection = AuthRejection;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let jar = CookieJar::from_headers(&parts.headers);
        let cookie_token = jar.get(AUTH_COOKIE_NAME).map(|c| c.value().to_string());

        let header_token = parts
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(|t| t.to_string());

        // If the Bearer token is an API key, use API key auth
        if let Some(ref token) = header_token
            && token.starts_with(API_KEY_PREFIX)
        {
            let api_key_auth =
                ApiKeyAuth::from_request_parts(parts, state)
                    .await
                    .map_err(|_| AuthRejection {
                        status: StatusCode::UNAUTHORIZED,
                        code: "INVALID_TOKEN",
                        message: "Invalid or expired authentication token",
                    })?;
            return Ok(Authenticated {
                user_id: api_key_auth.user_id,
                method: AuthMethod::ApiKey {
                    key_id: api_key_auth.key_id,
                    key_name: api_key_auth.key_name,
                    scopes: api_key_auth.scopes,
                    owner_is_admin: api_key_auth.owner_is_admin,
                },
            });
        }

        // Otherwise, try JWT (cookie first, then header)
        let token = cookie_token.or(header_token).ok_or(AuthRejection {
            status: StatusCode::UNAUTHORIZED,
            code: "MISSING_TOKEN",
            message: "No authentication token provided",
        })?;

        let jwt_config = Arc::<JwtConfig>::from_ref(state);
        let claims = AccessToken::decode(&token, &jwt_config).map_err(|_| AuthRejection {
            status: StatusCode::UNAUTHORIZED,
            code: "INVALID_TOKEN",
            message: "Invalid or expired authentication token",
        })?;

        Ok(Authenticated {
            user_id: claims.user_id,
            method: AuthMethod::Jwt {
                username: claims.username,
                is_admin: claims.is_admin,
            },
        })
    }
}
