//! In-memory [`Store`](crate::store::Store) implementation for development and testing.
//!
//! [`InMemoryStore`] uses `Arc<RwLock<..>>` internally, making it safe to share
//! across tasks. Data is lost when the process exits.
//!
//! # Examples
//!
//! ```no_run
//! use ironflow_store::prelude::*;
//! use serde_json::json;
//!
//! # async fn example() -> Result<(), ironflow_store::error::StoreError> {
//! let store = InMemoryStore::new();
//!
//! let run = store.create_run(NewRun {
//!     workflow_name: "test".to_string(),
//!     trigger: TriggerKind::Manual,
//!     payload: json!({}),
//!     max_retries: 3,
//!     handler_version: None,
//! }).await?;
//!
//! assert_eq!(run.status.state, RunStatus::Pending);
//! # Ok(())
//! # }
//! ```

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;
use uuid::Uuid;

use crate::entities::User;

mod api_key_store;
mod run_store;
mod secret_store;
mod user_store;

#[derive(Debug, Default)]
pub(super) struct State {
    pub(super) runs: HashMap<Uuid, crate::entities::Run>,
    pub(super) steps: HashMap<Uuid, crate::entities::Step>,
    pub(super) step_dependencies: Vec<crate::entities::StepDependency>,
    pub(super) users: HashMap<Uuid, User>,
    pub(super) api_keys: HashMap<Uuid, crate::entities::ApiKey>,
    pub(super) secrets: HashMap<String, EncryptedSecret>,
}

#[derive(Debug, Clone)]
pub(super) struct EncryptedSecret {
    pub(super) id: Uuid,
    pub(super) key: String,
    #[cfg(feature = "secret-store")]
    pub(super) encrypted_value: Vec<u8>,
    #[cfg(feature = "secret-store")]
    pub(super) nonce: Vec<u8>,
    pub(super) created_at: chrono::DateTime<chrono::Utc>,
    pub(super) updated_at: chrono::DateTime<chrono::Utc>,
}

/// In-memory store backed by `Arc<RwLock<..>>`.
///
/// Thread-safe and cheap to clone. All data is held in memory and lost on drop.
/// Implements [`Store`](crate::store::Store) so a single `Arc<InMemoryStore>`
/// covers runs, users, API keys, and secrets.
///
/// # Examples
///
/// ```
/// use ironflow_store::memory::InMemoryStore;
///
/// let store = InMemoryStore::new();
/// let store2 = store.clone(); // cheap Arc clone
/// ```
#[derive(Debug, Clone)]
pub struct InMemoryStore {
    pub(super) state: Arc<RwLock<State>>,
    #[cfg(feature = "secret-store")]
    pub(super) master_key: Option<Arc<crate::crypto::MasterKey>>,
}

impl InMemoryStore {
    /// Create a new empty in-memory store.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_store::memory::InMemoryStore;
    ///
    /// let store = InMemoryStore::new();
    /// ```
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(State::default())),
            #[cfg(feature = "secret-store")]
            master_key: None,
        }
    }

    /// Set the master key for secret encryption.
    ///
    /// Required before using [`SecretStore`](crate::secret_store::SecretStore)
    /// methods that read/write secret values. Without a master key, those
    /// methods return [`StoreError::Crypto`](crate::error::StoreError::Crypto).
    ///
    /// Listing and deleting secrets works without a master key.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_store::memory::InMemoryStore;
    /// use ironflow_store::crypto::MasterKey;
    ///
    /// # fn example() -> Result<(), ironflow_store::crypto::CryptoError> {
    /// let mut store = InMemoryStore::new();
    /// let key = MasterKey::from_hex(
    ///     "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
    /// )?;
    /// store.set_master_key(key);
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(feature = "secret-store")]
    pub fn set_master_key(&mut self, key: crate::crypto::MasterKey) {
        self.master_key = Some(Arc::new(key));
    }
}

impl Default for InMemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::entities::{NewRun, TriggerKind};

    pub(crate) fn new_run_req(name: &str) -> NewRun {
        NewRun {
            workflow_name: name.to_string(),
            trigger: TriggerKind::Manual,
            payload: json!({}),
            max_retries: 3,
            handler_version: None,
        }
    }
}
