//! Per-scope HKDF-SHA256 key derivation with AES-256-GCM encryption.
//!
//! [`WorkflowSecrets`] derives a unique encryption key for each scope
//! (workflow, operation, or system-wide) from a single master key using
//! HKDF-SHA256 (RFC 5869). Each derived key encrypts with AES-256-GCM and
//! a fresh OS-random nonce.
//!
//! The master key is read from the `IRONFLOW_SECRETS_KEY` environment variable
//! (base64-encoded 32 bytes).
//!
//! [`ScopedSecretStore`] combines scope-based key prefixing with any
//! [`SecretStore`] implementation for CRUD operations isolated per scope.
//!
//! # Wire format
//!
//! `base64(12-byte-nonce || ciphertext)`
//!
//! The same plaintext encrypted twice produces different ciphertexts because
//! the nonce is fresh every time.
//!
//! # Examples
//!
//! ```no_run
//! use ironflow_store::workflow_secrets::WorkflowSecrets;
//! use uuid::Uuid;
//!
//! let wf_id = Uuid::now_v7();
//! let secrets = WorkflowSecrets::for_workflow(wf_id);
//!
//! let encrypted = secrets.encrypt("sk-ant-api-key");
//! let decrypted = secrets.decrypt(&encrypted).unwrap();
//! assert_eq!(decrypted, "sk-ant-api-key");
//! ```

use std::env;
use std::fmt;
use std::sync::Arc;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use hkdf::Hkdf;
use rand::RngCore;
use sha2::Sha256;
use thiserror::Error;
use uuid::Uuid;

use crate::entities::{Page, Secret, SecretMetadata};
use crate::secret_store::SecretStore;

/// Environment variable holding the base64-encoded 32-byte master key.
pub const SECRETS_KEY_ENV: &str = "IRONFLOW_SECRETS_KEY";

/// HKDF info prefix for workflow-scoped secrets.
const WORKFLOW_SCOPE_PREFIX: &[u8] = b"ironflow-workflow:";

/// HKDF info prefix for operation-scoped secrets.
const OPERATION_SCOPE_PREFIX: &[u8] = b"ironflow-operation:";

/// HKDF info for system-wide secrets.
const SYSTEM_SCOPE: &[u8] = b"ironflow-system";

/// Errors from [`WorkflowSecrets`] operations.
///
/// # Examples
///
/// ```
/// use ironflow_store::workflow_secrets::SecretsError;
///
/// let err = SecretsError::MissingKey;
/// assert!(err.to_string().contains("not configured"));
/// ```
#[derive(Debug, Error)]
pub enum SecretsError {
    /// The master key environment variable is not set.
    #[error("encryption key not configured (set {SECRETS_KEY_ENV})")]
    MissingKey,

    /// The master key is not valid base64 or is not exactly 32 bytes.
    #[error("invalid key length (expected 32 bytes)")]
    InvalidKeyLength,

    /// Decryption failed (wrong scope, corrupted data, or invalid encoding).
    #[error("decryption failed: {0}")]
    DecryptionFailed(String),
}

/// Per-scope AES-256-GCM cipher derived from the master key via HKDF-SHA256.
///
/// Each scope (workflow, operation, system) gets a unique derived key, so
/// ciphertext produced under one scope cannot be decrypted under another.
///
/// Use the panicking constructors ([`for_workflow`](Self::for_workflow),
/// [`for_operation`](Self::for_operation), [`for_system`](Self::for_system))
/// at startup, and the fallible [`try_for_*`](Self::try_for_workflow)
/// variants in request handlers.
///
/// # Examples
///
/// ```no_run
/// use ironflow_store::workflow_secrets::WorkflowSecrets;
/// use uuid::Uuid;
///
/// // Panicking constructor for startup:
/// let secrets = WorkflowSecrets::for_system();
///
/// // Fallible constructor for request paths:
/// let wf_id = Uuid::now_v7();
/// let secrets = WorkflowSecrets::try_for_workflow(wf_id).expect("key configured");
/// ```
pub struct WorkflowSecrets {
    cipher: Aes256Gcm,
}

impl WorkflowSecrets {
    /// Derive a cipher scoped to a workflow.
    ///
    /// # Panics
    ///
    /// Panics if `IRONFLOW_SECRETS_KEY` is not set or invalid.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_store::workflow_secrets::WorkflowSecrets;
    /// use uuid::Uuid;
    ///
    /// let secrets = WorkflowSecrets::for_workflow(Uuid::now_v7());
    /// ```
    pub fn for_workflow(workflow_id: Uuid) -> Self {
        Self::derive(&scoped_info(WORKFLOW_SCOPE_PREFIX, workflow_id.as_bytes()))
    }

    /// Derive a cipher scoped to an operation.
    ///
    /// # Panics
    ///
    /// Panics if `IRONFLOW_SECRETS_KEY` is not set or invalid.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_store::workflow_secrets::WorkflowSecrets;
    /// use uuid::Uuid;
    ///
    /// let secrets = WorkflowSecrets::for_operation(Uuid::now_v7());
    /// ```
    pub fn for_operation(operation_id: Uuid) -> Self {
        Self::derive(&scoped_info(
            OPERATION_SCOPE_PREFIX,
            operation_id.as_bytes(),
        ))
    }

    /// Derive a cipher for system-wide secrets.
    ///
    /// # Panics
    ///
    /// Panics if `IRONFLOW_SECRETS_KEY` is not set or invalid.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_store::workflow_secrets::WorkflowSecrets;
    ///
    /// let secrets = WorkflowSecrets::for_system();
    /// ```
    pub fn for_system() -> Self {
        Self::derive(SYSTEM_SCOPE)
    }

    /// Fallible version of [`for_workflow`](Self::for_workflow).
    ///
    /// # Errors
    ///
    /// Returns [`SecretsError::MissingKey`] or [`SecretsError::InvalidKeyLength`].
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_store::workflow_secrets::WorkflowSecrets;
    /// use uuid::Uuid;
    ///
    /// # fn example() -> Result<(), ironflow_store::workflow_secrets::SecretsError> {
    /// let secrets = WorkflowSecrets::try_for_workflow(Uuid::now_v7())?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn try_for_workflow(workflow_id: Uuid) -> Result<Self, SecretsError> {
        Self::try_derive(&scoped_info(WORKFLOW_SCOPE_PREFIX, workflow_id.as_bytes()))
    }

    /// Fallible version of [`for_operation`](Self::for_operation).
    ///
    /// # Errors
    ///
    /// Returns [`SecretsError::MissingKey`] or [`SecretsError::InvalidKeyLength`].
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_store::workflow_secrets::WorkflowSecrets;
    /// use uuid::Uuid;
    ///
    /// # fn example() -> Result<(), ironflow_store::workflow_secrets::SecretsError> {
    /// let secrets = WorkflowSecrets::try_for_operation(Uuid::now_v7())?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn try_for_operation(operation_id: Uuid) -> Result<Self, SecretsError> {
        Self::try_derive(&scoped_info(
            OPERATION_SCOPE_PREFIX,
            operation_id.as_bytes(),
        ))
    }

    /// Fallible version of [`for_system`](Self::for_system).
    ///
    /// # Errors
    ///
    /// Returns [`SecretsError::MissingKey`] or [`SecretsError::InvalidKeyLength`].
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_store::workflow_secrets::WorkflowSecrets;
    ///
    /// # fn example() -> Result<(), ironflow_store::workflow_secrets::SecretsError> {
    /// let secrets = WorkflowSecrets::try_for_system()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn try_for_system() -> Result<Self, SecretsError> {
        Self::try_derive(SYSTEM_SCOPE)
    }

    /// Encrypt a plaintext string.
    ///
    /// Returns `base64(12-byte-nonce || ciphertext)`. A fresh OS-random nonce
    /// is generated for every call, so the same plaintext produces different
    /// output each time.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_store::workflow_secrets::WorkflowSecrets;
    ///
    /// let secrets = WorkflowSecrets::for_system();
    /// let ct1 = secrets.encrypt("api-key");
    /// let ct2 = secrets.encrypt("api-key");
    /// assert_ne!(ct1, ct2); // fresh nonce each time
    /// ```
    pub fn encrypt(&self, plaintext: &str) -> String {
        let mut nonce_bytes = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = self
            .cipher
            .encrypt(nonce, plaintext.as_bytes())
            .expect("AES-256-GCM encryption should not fail with valid key");

        let mut combined = nonce_bytes.to_vec();
        combined.extend_from_slice(&ciphertext);
        BASE64.encode(&combined)
    }

    /// Decrypt a value previously encrypted with the same scope.
    ///
    /// # Errors
    ///
    /// Returns [`SecretsError::DecryptionFailed`] if the base64 is invalid,
    /// the ciphertext is too short, the scope does not match, or the data
    /// has been tampered with.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_store::workflow_secrets::WorkflowSecrets;
    ///
    /// # fn example() -> Result<(), ironflow_store::workflow_secrets::SecretsError> {
    /// let secrets = WorkflowSecrets::for_system();
    /// let encrypted = secrets.encrypt("my-secret");
    /// let plaintext = secrets.decrypt(&encrypted)?;
    /// assert_eq!(plaintext, "my-secret");
    /// # Ok(())
    /// # }
    /// ```
    pub fn decrypt(&self, encrypted: &str) -> Result<String, SecretsError> {
        let combined = BASE64
            .decode(encrypted)
            .map_err(|e| SecretsError::DecryptionFailed(format!("invalid base64: {e}")))?;

        if combined.len() < 12 {
            return Err(SecretsError::DecryptionFailed(
                "ciphertext too short".to_string(),
            ));
        }

        let (nonce_bytes, ciphertext) = combined.split_at(12);
        let nonce = Nonce::from_slice(nonce_bytes);

        let plaintext = self
            .cipher
            .decrypt(nonce, ciphertext)
            .map_err(|_| SecretsError::DecryptionFailed("decryption failed".to_string()))?;

        String::from_utf8(plaintext)
            .map_err(|e| SecretsError::DecryptionFailed(format!("invalid UTF-8: {e}")))
    }

    fn derive(info: &[u8]) -> Self {
        Self::derive_with_master(info, &load_master_key())
    }

    fn try_derive(info: &[u8]) -> Result<Self, SecretsError> {
        Ok(Self::derive_with_master(info, &try_load_master_key()?))
    }

    fn derive_with_master(info: &[u8], master: &[u8]) -> Self {
        let hk = Hkdf::<Sha256>::new(None, master);
        let mut derived = [0u8; 32];
        hk.expand(info, &mut derived)
            .expect("HKDF expand cannot fail with 32-byte output");
        let cipher = Aes256Gcm::new_from_slice(&derived).expect("derived key is always 32 bytes");
        Self { cipher }
    }

    #[cfg(test)]
    fn for_workflow_with_master(workflow_id: Uuid, master: &[u8]) -> Self {
        Self::derive_with_master(
            &scoped_info(WORKFLOW_SCOPE_PREFIX, workflow_id.as_bytes()),
            master,
        )
    }

    #[cfg(test)]
    fn for_operation_with_master(operation_id: Uuid, master: &[u8]) -> Self {
        Self::derive_with_master(
            &scoped_info(OPERATION_SCOPE_PREFIX, operation_id.as_bytes()),
            master,
        )
    }

    #[cfg(test)]
    fn for_system_with_master(master: &[u8]) -> Self {
        Self::derive_with_master(SYSTEM_SCOPE, master)
    }
}

impl fmt::Debug for WorkflowSecrets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("WorkflowSecrets(***)")
    }
}

fn scoped_info(prefix: &[u8], id: &[u8]) -> Vec<u8> {
    let mut info = Vec::with_capacity(prefix.len() + id.len());
    info.extend_from_slice(prefix);
    info.extend_from_slice(id);
    info
}

fn try_load_master_key() -> Result<Vec<u8>, SecretsError> {
    let b64 = env::var(SECRETS_KEY_ENV)
        .ok()
        .filter(|s| !s.is_empty())
        .ok_or(SecretsError::MissingKey)?;

    let bytes = BASE64
        .decode(&b64)
        .map_err(|_| SecretsError::InvalidKeyLength)?;

    if bytes.len() != 32 {
        return Err(SecretsError::InvalidKeyLength);
    }

    Ok(bytes)
}

fn load_master_key() -> Vec<u8> {
    try_load_master_key().expect("IRONFLOW_SECRETS_KEY must be set (base64-encoded 32-byte key)")
}

/// Scoped view of a [`SecretStore`] that prefixes keys by scope.
///
/// All keys are automatically prefixed with the scope identifier
/// (e.g. `workflows/<uuid>/` or `operations/<uuid>/` or `system/`),
/// isolating secrets between different workflows or operations.
///
/// # Examples
///
/// ```no_run
/// use ironflow_store::workflow_secrets::ScopedSecretStore;
/// use ironflow_store::prelude::*;
/// use uuid::Uuid;
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_store::error::StoreError> {
/// let store = Arc::new(InMemoryStore::new());
/// let wf_id = Uuid::now_v7();
/// let scoped = ScopedSecretStore::for_workflow(wf_id, store);
///
/// // Key is stored as "workflows/<uuid>/api_token" internally.
/// scoped.set("api_token", "sk-ant-...").await?;
///
/// let secret = scoped.get("api_token").await?;
/// assert!(secret.is_some());
/// # Ok(())
/// # }
/// ```
pub struct ScopedSecretStore {
    prefix: String,
    store: Arc<dyn SecretStore>,
}

impl ScopedSecretStore {
    /// Create a scoped store for a workflow.
    ///
    /// Keys are prefixed with `workflows/<uuid>/`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_store::workflow_secrets::ScopedSecretStore;
    /// use ironflow_store::prelude::*;
    /// use uuid::Uuid;
    /// use std::sync::Arc;
    ///
    /// let store = Arc::new(InMemoryStore::new());
    /// let scoped = ScopedSecretStore::for_workflow(Uuid::now_v7(), store);
    /// ```
    pub fn for_workflow(workflow_id: Uuid, store: Arc<dyn SecretStore>) -> Self {
        Self {
            prefix: format!("workflows/{workflow_id}/"),
            store,
        }
    }

    /// Create a scoped store for an operation.
    ///
    /// Keys are prefixed with `operations/<uuid>/`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_store::workflow_secrets::ScopedSecretStore;
    /// use ironflow_store::prelude::*;
    /// use uuid::Uuid;
    /// use std::sync::Arc;
    ///
    /// let store = Arc::new(InMemoryStore::new());
    /// let scoped = ScopedSecretStore::for_operation(Uuid::now_v7(), store);
    /// ```
    pub fn for_operation(operation_id: Uuid, store: Arc<dyn SecretStore>) -> Self {
        Self {
            prefix: format!("operations/{operation_id}/"),
            store,
        }
    }

    /// Create a scoped store for system-wide secrets.
    ///
    /// Keys are prefixed with `system/`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_store::workflow_secrets::ScopedSecretStore;
    /// use ironflow_store::prelude::*;
    /// use std::sync::Arc;
    ///
    /// let store = Arc::new(InMemoryStore::new());
    /// let scoped = ScopedSecretStore::for_system(store);
    /// ```
    pub fn for_system(store: Arc<dyn SecretStore>) -> Self {
        Self {
            prefix: "system/".to_string(),
            store,
        }
    }

    /// Store an encrypted secret under the scoped key.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`](crate::error::StoreError) if encryption or
    /// storage fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_store::workflow_secrets::ScopedSecretStore;
    /// use ironflow_store::prelude::*;
    /// use uuid::Uuid;
    /// use std::sync::Arc;
    ///
    /// # async fn example() -> Result<(), ironflow_store::error::StoreError> {
    /// let store = Arc::new(InMemoryStore::new());
    /// let scoped = ScopedSecretStore::for_workflow(Uuid::now_v7(), store);
    /// scoped.set("api_key", "sk-ant-12345").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn set(&self, key: &str, value: &str) -> Result<Secret, crate::error::StoreError> {
        let full_key = format!("{}{key}", self.prefix);
        self.store.set_secret(&full_key, value).await
    }

    /// Retrieve a secret by scoped key.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`](crate::error::StoreError) if decryption fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_store::workflow_secrets::ScopedSecretStore;
    /// use ironflow_store::prelude::*;
    /// use uuid::Uuid;
    /// use std::sync::Arc;
    ///
    /// # async fn example() -> Result<(), ironflow_store::error::StoreError> {
    /// let store = Arc::new(InMemoryStore::new());
    /// let scoped = ScopedSecretStore::for_workflow(Uuid::now_v7(), store);
    /// let secret = scoped.get("api_key").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get(&self, key: &str) -> Result<Option<Secret>, crate::error::StoreError> {
        let full_key = format!("{}{key}", self.prefix);
        self.store.get_secret(&full_key).await
    }

    /// Delete a secret by scoped key.
    ///
    /// Returns `true` if the secret existed and was deleted.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`](crate::error::StoreError) on storage failure.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_store::workflow_secrets::ScopedSecretStore;
    /// use ironflow_store::prelude::*;
    /// use uuid::Uuid;
    /// use std::sync::Arc;
    ///
    /// # async fn example() -> Result<(), ironflow_store::error::StoreError> {
    /// let store = Arc::new(InMemoryStore::new());
    /// let scoped = ScopedSecretStore::for_workflow(Uuid::now_v7(), store);
    /// let deleted = scoped.delete("api_key").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn delete(&self, key: &str) -> Result<bool, crate::error::StoreError> {
        let full_key = format!("{}{key}", self.prefix);
        self.store.delete_secret(&full_key).await
    }

    /// List secret keys in this scope.
    ///
    /// Returns the full keys including the scope prefix.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`](crate::error::StoreError) on storage failure.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_store::workflow_secrets::ScopedSecretStore;
    /// use ironflow_store::prelude::*;
    /// use uuid::Uuid;
    /// use std::sync::Arc;
    ///
    /// # async fn example() -> Result<(), ironflow_store::error::StoreError> {
    /// let store = Arc::new(InMemoryStore::new());
    /// let scoped = ScopedSecretStore::for_workflow(Uuid::now_v7(), store);
    /// let keys = scoped.list_keys().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn list_keys(&self) -> Result<Vec<String>, crate::error::StoreError> {
        self.store.list_secret_keys(&self.prefix).await
    }

    /// List secret metadata in this scope, with pagination.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`](crate::error::StoreError) on storage failure.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_store::workflow_secrets::ScopedSecretStore;
    /// use ironflow_store::prelude::*;
    /// use uuid::Uuid;
    /// use std::sync::Arc;
    ///
    /// # async fn example() -> Result<(), ironflow_store::error::StoreError> {
    /// let store = Arc::new(InMemoryStore::new());
    /// let scoped = ScopedSecretStore::for_workflow(Uuid::now_v7(), store);
    /// let page = scoped.list(1, 25).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn list(
        &self,
        page: u32,
        per_page: u32,
    ) -> Result<Page<SecretMetadata>, crate::error::StoreError> {
        self.store.list_secrets(&self.prefix, page, per_page).await
    }
}

impl fmt::Debug for ScopedSecretStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScopedSecretStore")
            .field("prefix", &self.prefix)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MASTER: [u8; 32] = [1u8; 32];
    const UUID_A: &str = "11111111-1111-1111-1111-111111111111";
    const UUID_B: &str = "22222222-2222-2222-2222-222222222222";

    fn uuid_a() -> Uuid {
        Uuid::parse_str(UUID_A).unwrap()
    }

    fn uuid_b() -> Uuid {
        Uuid::parse_str(UUID_B).unwrap()
    }

    #[test]
    fn round_trip_workflow() {
        let secrets = WorkflowSecrets::for_workflow_with_master(uuid_a(), &MASTER);
        let encrypted = secrets.encrypt("workflow-api-key");
        let decrypted = secrets.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, "workflow-api-key");
    }

    #[test]
    fn round_trip_operation() {
        let secrets = WorkflowSecrets::for_operation_with_master(uuid_a(), &MASTER);
        let encrypted = secrets.encrypt("operation-token");
        let decrypted = secrets.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, "operation-token");
    }

    #[test]
    fn round_trip_system() {
        let secrets = WorkflowSecrets::for_system_with_master(&MASTER);
        let encrypted = secrets.encrypt("system-secret");
        let decrypted = secrets.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, "system-secret");
    }

    #[test]
    fn different_scope_cannot_decrypt() {
        let a = WorkflowSecrets::for_workflow_with_master(uuid_a(), &MASTER);
        let b = WorkflowSecrets::for_workflow_with_master(uuid_b(), &MASTER);
        let encrypted = a.encrypt("secret");
        assert!(b.decrypt(&encrypted).is_err());
    }

    #[test]
    fn same_plaintext_different_ciphertext() {
        let secrets = WorkflowSecrets::for_system_with_master(&MASTER);
        let ct1 = secrets.encrypt("same-value");
        let ct2 = secrets.encrypt("same-value");
        assert_ne!(ct1, ct2);
        assert_eq!(secrets.decrypt(&ct1).unwrap(), "same-value");
        assert_eq!(secrets.decrypt(&ct2).unwrap(), "same-value");
    }

    #[test]
    fn missing_key_error() {
        let saved = env::var(SECRETS_KEY_ENV).ok();
        unsafe { env::remove_var(SECRETS_KEY_ENV) };

        let result = WorkflowSecrets::try_for_workflow(uuid_a());
        assert!(matches!(result.unwrap_err(), SecretsError::MissingKey));

        if let Some(val) = saved {
            unsafe { env::set_var(SECRETS_KEY_ENV, val) };
        }
    }

    #[test]
    fn invalid_key_length() {
        let saved = env::var(SECRETS_KEY_ENV).ok();
        unsafe { env::set_var(SECRETS_KEY_ENV, BASE64.encode([0u8; 16])) };

        let result = WorkflowSecrets::try_for_workflow(uuid_a());
        assert!(matches!(
            result.unwrap_err(),
            SecretsError::InvalidKeyLength
        ));

        match saved {
            Some(val) => unsafe { env::set_var(SECRETS_KEY_ENV, val) },
            None => unsafe { env::remove_var(SECRETS_KEY_ENV) },
        }
    }

    #[test]
    fn invalid_base64_decrypt() {
        let secrets = WorkflowSecrets::for_system_with_master(&MASTER);
        let result = secrets.decrypt("not-valid-base64!!!");
        assert!(matches!(
            result.unwrap_err(),
            SecretsError::DecryptionFailed(_)
        ));
    }

    #[test]
    fn short_ciphertext_decrypt() {
        let secrets = WorkflowSecrets::for_system_with_master(&MASTER);
        let short = BASE64.encode([0u8; 5]);
        let result = secrets.decrypt(&short);
        match result.unwrap_err() {
            SecretsError::DecryptionFailed(msg) => {
                assert!(msg.contains("too short"));
            }
            other => panic!("expected DecryptionFailed, got {other:?}"),
        }
    }

    #[test]
    fn workflow_and_operation_scopes_are_isolated() {
        let wf = WorkflowSecrets::for_workflow_with_master(uuid_a(), &MASTER);
        let op = WorkflowSecrets::for_operation_with_master(uuid_a(), &MASTER);
        let encrypted = wf.encrypt("cross-scope");
        assert!(op.decrypt(&encrypted).is_err());
    }

    #[test]
    fn debug_redacts_cipher() {
        let secrets = WorkflowSecrets::for_system_with_master(&MASTER);
        let debug = format!("{secrets:?}");
        assert_eq!(debug, "WorkflowSecrets(***)");
    }

    #[cfg(feature = "secret-store")]
    mod scoped_store_tests {
        use super::*;
        use crate::crypto::{KeyRing, MasterKey};
        use crate::memory::InMemoryStore;

        fn test_store() -> Arc<InMemoryStore> {
            let mut store = InMemoryStore::new();
            let key = MasterKey::from_bytes(&[7u8; 32]).unwrap();
            let ring = KeyRing::single(key);
            store.set_key_ring(ring);
            Arc::new(store)
        }

        #[tokio::test]
        async fn scoped_store_set_get() {
            let store = test_store();
            let scoped = ScopedSecretStore::for_workflow(uuid_a(), store);

            let secret = scoped.set("api_token", "sk-ant-12345").await.unwrap();
            assert!(secret.key.starts_with("workflows/"));
            assert!(secret.key.ends_with("/api_token"));

            let retrieved = scoped.get("api_token").await.unwrap().unwrap();
            assert_eq!(retrieved.value, "sk-ant-12345");
        }

        #[tokio::test]
        async fn scoped_store_list_keys() {
            let store = test_store();
            let wf_a = ScopedSecretStore::for_workflow(uuid_a(), store.clone());
            let wf_b = ScopedSecretStore::for_workflow(uuid_b(), store);

            wf_a.set("key1", "val1").await.unwrap();
            wf_a.set("key2", "val2").await.unwrap();
            wf_b.set("key3", "val3").await.unwrap();

            let keys_a = wf_a.list_keys().await.unwrap();
            assert_eq!(keys_a.len(), 2);
            assert!(keys_a.iter().all(|k| k.contains(&uuid_a().to_string())));

            let keys_b = wf_b.list_keys().await.unwrap();
            assert_eq!(keys_b.len(), 1);
        }

        #[tokio::test]
        async fn scoped_store_delete() {
            let store = test_store();
            let scoped = ScopedSecretStore::for_workflow(uuid_a(), store);

            scoped.set("to_delete", "value").await.unwrap();
            assert!(scoped.get("to_delete").await.unwrap().is_some());

            let deleted = scoped.delete("to_delete").await.unwrap();
            assert!(deleted);

            assert!(scoped.get("to_delete").await.unwrap().is_none());
        }

        #[tokio::test]
        async fn scoped_store_isolation_between_scopes() {
            let store = test_store();
            let wf = ScopedSecretStore::for_workflow(uuid_a(), store.clone());
            let op = ScopedSecretStore::for_operation(uuid_a(), store.clone());
            let sys = ScopedSecretStore::for_system(store);

            wf.set("shared_name", "wf_value").await.unwrap();
            op.set("shared_name", "op_value").await.unwrap();
            sys.set("shared_name", "sys_value").await.unwrap();

            assert_eq!(
                wf.get("shared_name").await.unwrap().unwrap().value,
                "wf_value"
            );
            assert_eq!(
                op.get("shared_name").await.unwrap().unwrap().value,
                "op_value"
            );
            assert_eq!(
                sys.get("shared_name").await.unwrap().unwrap().value,
                "sys_value"
            );
        }

        #[test]
        fn scoped_store_debug() {
            let store = test_store();
            let scoped = ScopedSecretStore::for_workflow(uuid_a(), store);
            let debug = format!("{scoped:?}");
            assert!(debug.contains("ScopedSecretStore"));
            assert!(debug.contains("workflows/"));
        }
    }
}
