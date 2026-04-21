//! Secret entity for encrypted key-value storage.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// An encrypted secret stored in the database.
///
/// The `value` field contains the **plaintext** after decryption.
/// Raw ciphertext and nonce are internal to the store implementations.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::Secret;
/// use chrono::Utc;
/// use uuid::Uuid;
///
/// let secret = Secret {
///     id: Uuid::now_v7(),
///     key: "workflows/inbox/gmail_refresh_token".to_string(),
///     value: "ya29.a0AfH6SM...".to_string(),
///     created_at: Utc::now(),
///     updated_at: Utc::now(),
/// };
/// assert!(secret.key.starts_with("workflows/"));
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Secret {
    /// Unique secret ID (UUID v7).
    pub id: Uuid,
    /// Unique key, typically namespaced (e.g. `workflows/<name>/<secret_name>`).
    pub key: String,
    /// Decrypted plaintext value.
    #[serde(skip_serializing)]
    pub value: String,
    /// When the secret was first created.
    pub created_at: DateTime<Utc>,
    /// When the secret was last updated.
    pub updated_at: DateTime<Utc>,
}

/// Metadata about a secret, without the decrypted value.
///
/// Used for listing secrets in the API/dashboard where the value
/// must never be exposed.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::SecretMetadata;
/// use chrono::Utc;
/// use uuid::Uuid;
///
/// let meta = SecretMetadata {
///     id: Uuid::now_v7(),
///     key: "workflows/inbox/gmail_refresh_token".to_string(),
///     created_at: Utc::now(),
///     updated_at: Utc::now(),
/// };
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretMetadata {
    /// Unique secret ID (UUID v7).
    pub id: Uuid,
    /// Unique key.
    pub key: String,
    /// When the secret was first created.
    pub created_at: DateTime<Utc>,
    /// When the secret was last updated.
    pub updated_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_serde_excludes_value() {
        let secret = Secret {
            id: Uuid::now_v7(),
            key: "test/my-secret".to_string(),
            value: "super-secret-value-should-not-appear".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let json = serde_json::to_string(&secret).expect("serialize");
        assert!(!json.contains("super-secret-value-should-not-appear"));
        assert!(json.contains("test/my-secret"));
    }

    #[test]
    fn secret_preserves_value_in_struct() {
        let secret = Secret {
            id: Uuid::now_v7(),
            key: "k".to_string(),
            value: "v".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        assert_eq!(secret.value, "v");
    }

    #[test]
    fn secret_metadata_serde_has_no_value() {
        let meta = SecretMetadata {
            id: Uuid::now_v7(),
            key: "my/key".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let json = serde_json::to_string(&meta).expect("serialize");
        assert!(json.contains("my/key"));
        assert!(!json.contains("value"));
    }
}
