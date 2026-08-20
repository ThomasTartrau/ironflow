//! API key entity for machine-to-machine authentication.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::entities::api_key_scope::ApiKeyScope;

/// A stored API key (hashed, never contains the raw secret).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKey {
    /// Unique API key ID (UUID v7).
    pub id: Uuid,
    /// Owner user ID.
    pub user_id: Uuid,
    /// Human-readable name for this key.
    pub name: String,
    /// Argon2id hash of the raw key (never exposed).
    #[serde(skip_serializing)]
    pub key_hash: String,
    /// First 8 characters of the raw key for identification.
    pub key_prefix: String,
    /// Scopes granted to this key.
    pub scopes: Vec<ApiKeyScope>,
    /// Whether this key is active.
    pub is_active: bool,
    /// Optional expiration date.
    pub expires_at: Option<DateTime<Utc>>,
    /// Last time this key was used.
    pub last_used_at: Option<DateTime<Utc>>,
    /// When the key was created.
    pub created_at: DateTime<Utc>,
    /// When the key was last updated.
    pub updated_at: DateTime<Utc>,
    /// Optional per-key rate limit override (requests per minute).
    ///
    /// When set, the API uses this value instead of the global rate limit
    /// for requests authenticated with this key. `None` means use the
    /// server default. `Some(0)` disables rate limiting for this key.
    pub rate_limit_override: Option<u32>,
}

/// Parameters for creating a new API key.
#[derive(Debug, Clone)]
pub struct NewApiKey {
    /// Owner user ID.
    pub user_id: Uuid,
    /// Human-readable name.
    pub name: String,
    /// Argon2id hash of the raw key.
    pub key_hash: String,
    /// First 8 characters of the raw key.
    pub key_prefix: String,
    /// Granted scopes.
    pub scopes: Vec<ApiKeyScope>,
    /// Optional expiration date.
    pub expires_at: Option<DateTime<Utc>>,
    /// Optional per-key rate limit override (requests per minute).
    /// `None` uses the server default. `Some(0)` disables rate limiting.
    pub rate_limit_override: Option<u32>,
}

/// Parameters for updating an API key.
#[derive(Debug, Clone, Default)]
pub struct ApiKeyUpdate {
    /// New name.
    pub name: Option<String>,
    /// New scopes.
    pub scopes: Option<Vec<ApiKeyScope>>,
    /// New active status.
    pub is_active: Option<bool>,
    /// New expiration date. `Some(None)` removes expiration.
    pub expires_at: Option<Option<DateTime<Utc>>>,
    /// New rate limit override. `Some(None)` removes the override.
    pub rate_limit_override: Option<Option<u32>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_key_serde_excludes_hash() {
        let key = ApiKey {
            id: Uuid::now_v7(),
            user_id: Uuid::now_v7(),
            name: "test-key".to_string(),
            key_hash: "secret_hash".to_string(),
            key_prefix: "irfl_abc".to_string(),
            scopes: vec![ApiKeyScope::RunsRead],
            is_active: true,
            expires_at: None,
            last_used_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            rate_limit_override: None,
        };

        let json = serde_json::to_string(&key).expect("serialize");
        assert!(!json.contains("secret_hash"));
        assert!(json.contains("test-key"));
        assert!(json.contains("irfl_abc"));
    }

    #[test]
    fn api_key_update_default_is_empty() {
        let update = ApiKeyUpdate::default();
        assert!(update.name.is_none());
        assert!(update.scopes.is_none());
        assert!(update.is_active.is_none());
        assert!(update.expires_at.is_none());
        assert!(update.rate_limit_override.is_none());
    }
}
