//! # ironflow-store
//!
//! Storage abstraction for the **ironflow** workflow engine. This crate provides
//! the [`RunStore`](store::RunStore) trait and data entities for persisting workflow
//! runs and steps.
//!
//! # Implementations
//!
//! | Store | Feature | Description |
//! |-------|---------|-------------|
//! | [`InMemoryStore`](memory::InMemoryStore) | `store-memory` (default) | In-process, no external dependencies. |
//! | [`PostgresStore`](postgres::PostgresStore) | `store-postgres` | Production-ready with `SELECT FOR UPDATE SKIP LOCKED`. |
//!
//! # Quick start
//!
//! ```no_run
//! use std::collections::HashMap;
//! use ironflow_store::prelude::*;
//! use serde_json::json;
//!
//! # async fn example() -> Result<(), ironflow_store::error::StoreError> {
//! let store = InMemoryStore::new();
//!
//! let run = store.create_run(NewRun {
//!     workflow_name: "deploy".to_string(),
//!     trigger: TriggerKind::Manual,
//!     payload: json!({}),
//!     max_retries: 3,
//!     handler_version: None,
//!     labels: HashMap::new(),
//!     scheduled_at: None,
//!     idempotency_key: None,
//! }).await?.into_run();
//!
//! println!("Run {} is {:?}", run.id, run.status);
//! # Ok(())
//! # }
//! ```

pub mod api_key_store;
pub mod audit_log_store;
pub mod entities;
pub mod error;
pub mod secret_store;
pub mod store;
pub mod user_store;

#[cfg(feature = "secret-store")]
pub mod crypto;

/// Backward-compatible alias -- prefer `entities` for new code.
pub use entities as models;

#[cfg(feature = "store-memory")]
pub mod memory;

#[cfg(feature = "store-postgres")]
pub mod postgres;

/// Convenience re-exports for common usage.
pub mod prelude {
    pub use crate::api_key_store::ApiKeyStore;
    pub use crate::audit_log_store::AuditLogStore;
    pub use crate::entities::*;
    pub use crate::error::StoreError;
    pub use crate::secret_store::SecretStore;
    pub use crate::store::{RunStore, Store};
    pub use crate::user_store::UserStore;

    #[cfg(feature = "store-memory")]
    pub use crate::memory::InMemoryStore;

    #[cfg(feature = "store-postgres")]
    pub use crate::postgres::PostgresStore;
}
