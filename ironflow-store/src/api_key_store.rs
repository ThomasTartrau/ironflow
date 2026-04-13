//! The [`ApiKeyStore`] trait -- async storage abstraction for API keys.

use uuid::Uuid;

use crate::entities::{ApiKey, ApiKeyUpdate, NewApiKey};
use crate::store::StoreFuture;

/// Async storage abstraction for API keys.
///
/// All methods return a [`StoreFuture`] (boxed future) for object safety,
/// allowing the store to be used as `Arc<dyn ApiKeyStore>`.
pub trait ApiKeyStore: Send + Sync {
    /// Create a new API key.
    fn create_api_key(&self, req: NewApiKey) -> StoreFuture<'_, ApiKey>;

    /// Find an API key by its prefix (first 8 chars).
    ///
    /// Returns all keys matching the prefix (should be at most one).
    /// The caller must verify the full key hash.
    fn find_api_key_by_prefix(&self, prefix: &str) -> StoreFuture<'_, Option<ApiKey>>;

    /// Find an API key by ID.
    fn find_api_key_by_id(&self, id: Uuid) -> StoreFuture<'_, Option<ApiKey>>;

    /// List all API keys for a user.
    fn list_api_keys_by_user(&self, user_id: Uuid) -> StoreFuture<'_, Vec<ApiKey>>;

    /// Update an API key.
    fn update_api_key(&self, id: Uuid, update: ApiKeyUpdate) -> StoreFuture<'_, ()>;

    /// Record that an API key was used (updates `last_used_at`).
    fn touch_api_key(&self, id: Uuid) -> StoreFuture<'_, ()>;

    /// Delete an API key.
    fn delete_api_key(&self, id: Uuid) -> StoreFuture<'_, ()>;
}
