//! User request and response DTOs.

use chrono::{DateTime, Utc};
use ironflow_store::entities::User;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use validator::Validate;

/// Response DTO for a user (never exposes password hash).
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Serialize)]
pub struct UserResponse {
    /// User ID.
    pub id: Uuid,
    /// Email address.
    pub email: String,
    /// Display username.
    pub username: String,
    /// Whether the user is an admin.
    pub is_admin: bool,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Last update timestamp.
    pub updated_at: DateTime<Utc>,
}

impl From<User> for UserResponse {
    fn from(user: User) -> Self {
        Self {
            id: user.id,
            email: user.email,
            username: user.username,
            is_admin: user.is_admin,
            created_at: user.created_at,
            updated_at: user.updated_at,
        }
    }
}

/// Request body for creating a user (admin only).
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Deserialize, Validate)]
pub struct CreateUserRequest {
    /// Email address.
    #[validate(email)]
    pub email: String,
    /// Display username.
    #[validate(length(min = 3, message = "username must be at least 3 characters"))]
    pub username: String,
    /// Plaintext password (min 8 characters).
    #[validate(length(min = 8, message = "password must be at least 8 characters"))]
    pub password: String,
    /// Whether the new user should be an admin.
    pub is_admin: bool,
}

/// Request body for updating a user's role (admin only).
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Deserialize)]
pub struct UpdateRoleRequest {
    /// New admin status.
    pub is_admin: bool,
}
