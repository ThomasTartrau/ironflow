//! Secret request and response DTOs.

use chrono::{DateTime, Utc};
use ironflow_store::entities::{Secret, SecretMetadata};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use validator::Validate;

/// Response DTO for a secret (never exposes the value).
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Serialize)]
pub struct SecretResponse {
    /// Secret ID.
    pub id: Uuid,
    /// Secret key.
    pub key: String,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Last update timestamp.
    pub updated_at: DateTime<Utc>,
}

impl From<SecretMetadata> for SecretResponse {
    fn from(meta: SecretMetadata) -> Self {
        Self {
            id: meta.id,
            key: meta.key,
            created_at: meta.created_at,
            updated_at: meta.updated_at,
        }
    }
}

impl From<Secret> for SecretResponse {
    fn from(secret: Secret) -> Self {
        Self {
            id: secret.id,
            key: secret.key,
            created_at: secret.created_at,
            updated_at: secret.updated_at,
        }
    }
}

/// Request body for creating or updating a secret.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Deserialize, Validate)]
pub struct SetSecretRequest {
    /// Secret key (namespaced, e.g. `workflows/inbox/gmail_token`).
    #[validate(length(min = 1, max = 512), custom(function = "validate_secret_key"))]
    pub key: String,
    /// Secret value (plaintext, will be encrypted at rest).
    #[validate(length(min = 1, max = 65536))]
    pub value: String,
}

/// Validate that a secret key contains only safe characters.
///
/// Allowed: alphanumeric, `/`, `-`, `_`, `.`
fn validate_secret_key(key: &str) -> Result<(), validator::ValidationError> {
    if !key
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '-' | '_' | '.'))
    {
        let mut err = validator::ValidationError::new("invalid_characters");
        err.message =
            Some("key must only contain alphanumeric characters, '/', '-', '_', '.'".into());
        return Err(err);
    }
    if key.starts_with('/') || key.ends_with('/') || key.contains("//") {
        let mut err = validator::ValidationError::new("invalid_format");
        err.message = Some("key must not start/end with '/' or contain '//'".into());
        return Err(err);
    }
    Ok(())
}
