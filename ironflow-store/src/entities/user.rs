//! User entity for IAM.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A registered user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    /// Unique user ID (UUID v7).
    pub id: Uuid,
    /// Email address (unique).
    pub email: String,
    /// Display username (unique).
    pub username: String,
    /// Argon2id password hash.
    #[serde(skip_serializing)]
    pub password_hash: String,
    /// Whether the user has admin privileges.
    pub is_admin: bool,
    /// When the user was created.
    pub created_at: DateTime<Utc>,
    /// When the user was last updated.
    pub updated_at: DateTime<Utc>,
}

/// Parameters for creating a new user.
#[derive(Debug, Clone)]
pub struct NewUser {
    /// Email address.
    pub email: String,
    /// Display username.
    pub username: String,
    /// Pre-hashed password (Argon2id).
    pub password_hash: String,
    /// Explicit admin flag. When `Some(true)`, the user is created as admin.
    /// When `None`, the store decides (first user = admin, others = member).
    pub is_admin: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_serde_excludes_password_hash_on_output() {
        let user = User {
            id: Uuid::now_v7(),
            email: "test@example.com".to_string(),
            username: "testuser".to_string(),
            password_hash: "secret_hash_should_not_appear".to_string(),
            is_admin: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let _json = serde_json::to_string(&user).expect("serialize");
        // Verify the password hash is not in the JSON output
        assert!(!_json.contains("secret_hash_should_not_appear"));
        assert!(_json.contains("test@example.com"));
        assert!(_json.contains("testuser"));
    }

    #[test]
    fn user_serde_with_explicit_password_hash() {
        let now = Utc::now();
        let user = User {
            id: Uuid::now_v7(),
            email: "alice@example.com".to_string(),
            username: "alice".to_string(),
            password_hash: "argon2_hash".to_string(),
            is_admin: true,
            created_at: now,
            updated_at: now,
        };

        // When serialized, password_hash is skipped
        let _json = serde_json::to_string(&user).expect("serialize");

        // But the struct itself preserves the hash internally
        assert_eq!(user.password_hash, "argon2_hash");
        assert_eq!(user.email, "alice@example.com");
        assert_eq!(user.username, "alice");
        assert!(user.is_admin);
    }

    #[test]
    fn newuser_basic_creation() {
        let new_user = NewUser {
            email: "bob@example.com".to_string(),
            username: "bob".to_string(),
            password_hash: "hash123".to_string(),
            is_admin: None,
        };

        assert_eq!(new_user.email, "bob@example.com");
        assert_eq!(new_user.username, "bob");
        assert_eq!(new_user.password_hash, "hash123");
        assert_eq!(new_user.is_admin, None);
    }
}
