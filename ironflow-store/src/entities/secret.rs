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

/// Default number of secrets re-encrypted per rotation batch.
pub const DEFAULT_ROTATION_BATCH_SIZE: u32 = 100;

/// Largest batch a single rotation call may process.
pub const MAX_ROTATION_BATCH_SIZE: u32 = 1000;

/// One batch of a key rotation.
///
/// Rotation is driven batch by batch by the caller rather than in one long
/// call, so that a rotation can be watched, interrupted, and resumed.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::RotationRequest;
///
/// // Start from the beginning of the stock.
/// let first = RotationRequest::new(2);
/// assert!(first.after_id.is_none());
///
/// // Continue after the last secret seen.
/// let next = first.clone().after(uuid::Uuid::now_v7());
/// assert!(next.after_id.is_some());
/// ```
#[derive(Debug, Clone)]
pub struct RotationRequest {
    /// Key version every secret in the batch is re-encrypted with.
    pub to_version: i32,
    /// How many secrets to process, clamped to
    /// `[1, MAX_ROTATION_BATCH_SIZE]`.
    pub batch_size: u32,
    /// Resume after this secret ID. `None` starts from the beginning.
    ///
    /// The cursor is what keeps the rotation moving forward when a secret
    /// cannot be decrypted: a failed row is never served again.
    pub after_id: Option<Uuid>,
}

impl RotationRequest {
    /// A request for the first batch, with the default batch size.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_store::entities::{RotationRequest, DEFAULT_ROTATION_BATCH_SIZE};
    ///
    /// let req = RotationRequest::new(2);
    /// assert_eq!(req.to_version, 2);
    /// assert_eq!(req.batch_size, DEFAULT_ROTATION_BATCH_SIZE);
    /// ```
    pub fn new(to_version: i32) -> Self {
        Self {
            to_version,
            batch_size: DEFAULT_ROTATION_BATCH_SIZE,
            after_id: None,
        }
    }

    /// Set the batch size.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_store::entities::RotationRequest;
    ///
    /// let req = RotationRequest::new(2).with_batch_size(10);
    /// assert_eq!(req.batch_size, 10);
    /// ```
    pub fn with_batch_size(mut self, batch_size: u32) -> Self {
        self.batch_size = batch_size;
        self
    }

    /// Resume after a given secret ID.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_store::entities::RotationRequest;
    /// use uuid::Uuid;
    ///
    /// let id = Uuid::now_v7();
    /// let req = RotationRequest::new(2).after(id);
    /// assert_eq!(req.after_id, Some(id));
    /// ```
    pub fn after(mut self, id: Uuid) -> Self {
        self.after_id = Some(id);
        self
    }

    /// The batch size clamped to the allowed range.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_store::entities::{RotationRequest, MAX_ROTATION_BATCH_SIZE};
    ///
    /// assert_eq!(RotationRequest::new(2).with_batch_size(0).effective_batch_size(), 1);
    /// assert_eq!(
    ///     RotationRequest::new(2).with_batch_size(99_999).effective_batch_size(),
    ///     MAX_ROTATION_BATCH_SIZE
    /// );
    /// ```
    pub fn effective_batch_size(&self) -> u32 {
        self.batch_size.clamp(1, MAX_ROTATION_BATCH_SIZE)
    }
}

/// Outcome of one rotation batch.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::RotationBatch;
///
/// let batch = RotationBatch {
///     to_version: 2,
///     rotated: 98,
///     failed: 2,
///     remaining: 348,
///     last_id: Some(uuid::Uuid::now_v7()),
/// };
/// assert!(!batch.is_complete());
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RotationBatch {
    /// Key version the batch re-encrypted towards.
    pub to_version: i32,
    /// Secrets successfully re-encrypted in this batch.
    pub rotated: u64,
    /// Secrets skipped because they could not be decrypted.
    pub failed: u64,
    /// Secrets left on another key version after this batch.
    pub remaining: u64,
    /// Highest secret ID seen in this batch, failures included.
    ///
    /// Pass it back as [`RotationRequest::after_id`] to continue. `None`
    /// means the batch was empty: there is nothing left to do.
    pub last_id: Option<Uuid>,
}

impl RotationBatch {
    /// Whether the rotation has nothing left to process.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_store::entities::RotationBatch;
    ///
    /// let done = RotationBatch {
    ///     to_version: 2,
    ///     rotated: 0,
    ///     failed: 0,
    ///     remaining: 0,
    ///     last_id: None,
    /// };
    /// assert!(done.is_complete());
    /// ```
    pub fn is_complete(&self) -> bool {
        self.last_id.is_none() || self.remaining == 0
    }
}

/// How the configured key ring lines up with what the stored secrets use.
///
/// This is what tells an operator whether an old key can be dropped from the
/// configuration, without having to guess.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::KeyVersionStatus;
///
/// let status = KeyVersionStatus {
///     active: 2,
///     configured: vec![1, 2],
///     in_use: vec![2],
///     missing: vec![],
///     retirable: vec![1],
/// };
/// assert!(status.is_consistent());
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyVersionStatus {
    /// Version used to encrypt new secrets.
    pub active: i32,
    /// Versions present in the configured key ring, ascending.
    pub configured: Vec<i32>,
    /// Versions actually used by stored secrets, ascending.
    pub in_use: Vec<i32>,
    /// Versions used by stored secrets but absent from the key ring.
    ///
    /// Non-empty means some secrets are unreadable: the server refuses to
    /// start in that state.
    pub missing: Vec<i32>,
    /// Versions that can be removed from the key ring safely: configured,
    /// not active, and unused by any secret.
    pub retirable: Vec<i32>,
}

impl KeyVersionStatus {
    /// Whether every stored secret can be decrypted with the current ring.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_store::entities::KeyVersionStatus;
    ///
    /// let broken = KeyVersionStatus {
    ///     active: 1,
    ///     configured: vec![1],
    ///     in_use: vec![1, 2],
    ///     missing: vec![2],
    ///     retirable: vec![],
    /// };
    /// assert!(!broken.is_consistent());
    /// ```
    pub fn is_consistent(&self) -> bool {
        self.missing.is_empty()
    }
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
