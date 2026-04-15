//! Auth request and response DTOs.

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use validator::Validate;

/// Sign-up request body.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Deserialize, Validate)]
pub struct SignUpRequest {
    /// Email address.
    #[validate(email)]
    pub email: String,
    /// Display username.
    #[validate(length(min = 3, message = "username must be at least 3 characters"))]
    pub username: String,
    /// Plaintext password (min 8 characters).
    #[validate(length(min = 8, message = "password must be at least 8 characters"))]
    pub password: String,
}

/// Sign-in request body.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Deserialize)]
pub struct SignInRequest {
    /// Email address.
    pub email: String,
    /// Plaintext password.
    pub password: String,
}

/// Current user profile response.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Serialize)]
pub struct MeResponse {
    /// User ID.
    pub user_id: Uuid,
    /// Email address.
    pub email: String,
    /// Display username.
    pub username: String,
    /// Admin flag.
    pub is_admin: bool,
}
