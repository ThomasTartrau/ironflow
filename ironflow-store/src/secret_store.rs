//! The [`SecretStore`] trait -- async storage abstraction for encrypted secrets.
//!
//! Provides CRUD operations on key-value secrets with encryption at rest.
//! Keys are typically namespaced by workflow (e.g. `workflows/inbox/gmail_refresh_token`).
//!
//! Built-in implementations:
//!
//! - [`InMemoryStore`](crate::memory::InMemoryStore) -- development and testing.
//! - `PostgresStore` -- production (behind the `store-postgres` feature).

use crate::entities::{Page, Secret, SecretMetadata};
use crate::store::StoreFuture;

/// Async storage abstraction for encrypted secrets.
///
/// All methods return a [`StoreFuture`] (boxed future) for object safety,
/// allowing the store to be used as `Arc<dyn SecretStore>`.
///
/// Values are encrypted at rest using AES-256-GCM. The master key is
/// provided by the caller at startup via `set_master_key()`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_store::prelude::*;
///
/// # async fn example(store: &dyn SecretStore) -> Result<(), ironflow_store::error::StoreError> {
/// store.set_secret("workflows/inbox/gmail_token", "ya29.a0AfH6SM...").await?;
///
/// let secret = store.get_secret("workflows/inbox/gmail_token").await?;
/// assert!(secret.is_some());
///
/// let keys = store.list_secret_keys("workflows/inbox/").await?;
/// assert_eq!(keys.len(), 1);
///
/// store.delete_secret("workflows/inbox/gmail_token").await?;
/// # Ok(())
/// # }
/// ```
pub trait SecretStore: Send + Sync {
    /// Retrieve a secret by key.
    ///
    /// Returns the decrypted [`Secret`] if found, `None` otherwise.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::StoreError::Crypto`] if decryption fails.
    fn get_secret(&self, key: &str) -> StoreFuture<'_, Option<Secret>>;

    /// Create or update a secret.
    ///
    /// If the key already exists, the value is re-encrypted and updated.
    /// If the key does not exist, a new secret is created.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::StoreError::Crypto`] if encryption fails.
    fn set_secret(&self, key: &str, value: &str) -> StoreFuture<'_, Secret>;

    /// Delete a secret by key.
    ///
    /// Returns `true` if the secret existed and was deleted, `false` if
    /// the key was not found.
    fn delete_secret(&self, key: &str) -> StoreFuture<'_, bool>;

    /// List all secret keys matching a prefix.
    ///
    /// Returns only the keys (not the values) to avoid unnecessary decryption.
    ///
    /// # Examples
    ///
    /// Keys like `workflows/inbox/token_a` and `workflows/inbox/token_b`
    /// are returned when calling `list_secret_keys("workflows/inbox/")`.
    fn list_secret_keys(&self, prefix: &str) -> StoreFuture<'_, Vec<String>>;

    /// List secret metadata matching a prefix, with pagination.
    ///
    /// Returns [`SecretMetadata`] entries (id, key, timestamps) without
    /// decrypting any values. Ordered by key ascending.
    ///
    /// Pass an empty prefix to list all secrets.
    fn list_secrets(
        &self,
        prefix: &str,
        page: u32,
        per_page: u32,
    ) -> StoreFuture<'_, Page<SecretMetadata>>;
}
