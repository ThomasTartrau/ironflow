//! Axum extractor for authenticated users.
//!
//! Extracts and validates a JWT from the request cookie or `Authorization: Bearer` header.

use std::sync::Arc;

use axum::Json;
use axum::extract::{FromRef, FromRequestParts};
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use axum_extra::extract::CookieJar;
use uuid::Uuid;

use crate::cookies::AUTH_COOKIE_NAME;
use crate::jwt::{AccessToken, JwtConfig};

/// An authenticated user extracted from a request.
///
/// Use as an Axum handler parameter to enforce authentication.
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

/// Rejection type when authentication fails.
pub struct AuthRejection {
    status: StatusCode,
    code: &'static str,
    message: &'static str,
}

impl IntoResponse for AuthRejection {
    fn into_response(self) -> Response {
        let body = serde_json::json!({
            "error": {
                "code": self.code,
                "message": self.message,
            }
        });
        (self.status, Json(body)).into_response()
    }
}

impl<S> FromRequestParts<S> for AuthenticatedUser
where
    S: Send + Sync,
    Arc<JwtConfig>: FromRef<S>,
{
    type Rejection = AuthRejection;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let jwt_config = Arc::<JwtConfig>::from_ref(state);

        // Try cookie first, then Authorization header
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
