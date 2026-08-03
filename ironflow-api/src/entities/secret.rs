//! Secret request and response DTOs.

use chrono::{DateTime, Utc};
use ironflow_store::entities::{
    DEFAULT_ROTATION_BATCH_SIZE, KeyVersionStatus, RotationBatch, Secret, SecretMetadata,
};
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

/// Request body for rotating a batch of secrets to another key version.
///
/// Rotation is driven one batch per call so a long rotation never becomes a
/// long HTTP request: the client loops, carrying `after_id` forward.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Default, Deserialize, Validate)]
pub struct RotateSecretsRequest {
    /// Target key version. Defaults to the server's active version.
    #[validate(range(min = 1))]
    pub to_version: Option<i32>,
    /// Secrets to process in this batch. Defaults to 100, clamped to 1000.
    #[validate(range(min = 1))]
    pub batch_size: Option<u32>,
    /// Resume after this secret ID. Omit to start from the beginning.
    pub after_id: Option<Uuid>,
}

impl RotateSecretsRequest {
    /// The batch size to apply, falling back to the default.
    pub fn effective_batch_size(&self) -> u32 {
        self.batch_size.unwrap_or(DEFAULT_ROTATION_BATCH_SIZE)
    }
}

/// Outcome of one rotation batch.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Serialize)]
pub struct RotateSecretsResponse {
    /// Key version the batch re-encrypted towards.
    pub to_version: i32,
    /// Secrets successfully re-encrypted in this batch.
    pub rotated: u64,
    /// Secrets skipped because they could not be decrypted.
    pub failed: u64,
    /// Secrets left on another key version after this batch.
    pub remaining: u64,
    /// Highest secret ID seen in this batch. Pass it back as `after_id` to
    /// continue; `null` means there is nothing left to do.
    pub last_id: Option<Uuid>,
}

impl From<RotationBatch> for RotateSecretsResponse {
    fn from(batch: RotationBatch) -> Self {
        Self {
            to_version: batch.to_version,
            rotated: batch.rotated,
            failed: batch.failed,
            remaining: batch.remaining,
            last_id: batch.last_id,
        }
    }
}

/// How the configured key ring lines up with the stored secrets.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Serialize)]
pub struct KeyVersionsResponse {
    /// Version used to encrypt new secrets.
    pub active: i32,
    /// Versions present in the configured key ring.
    pub configured: Vec<i32>,
    /// Versions actually used by stored secrets.
    pub in_use: Vec<i32>,
    /// Versions used by stored secrets but absent from the key ring.
    /// Non-empty means some secrets are unreadable.
    pub missing: Vec<i32>,
    /// Versions that can be removed from the key ring safely.
    pub retirable: Vec<i32>,
}

impl From<KeyVersionStatus> for KeyVersionsResponse {
    fn from(status: KeyVersionStatus) -> Self {
        Self {
            active: status.active,
            configured: status.configured,
            in_use: status.in_use,
            missing: status.missing,
            retirable: status.retirable,
        }
    }
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
