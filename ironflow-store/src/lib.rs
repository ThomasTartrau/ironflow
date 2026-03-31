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
//! }).await?;
//!
//! println!("Run {} is {:?}", run.id, run.status);
//! # Ok(())
//! # }
//! ```

pub mod entities;
pub mod error;
pub mod store;
pub mod user_store;

/// Backward-compatible alias — prefer `entities` for new code.
pub use entities as models;

#[cfg(feature = "store-memory")]
pub mod memory;

#[cfg(feature = "store-postgres")]
pub mod postgres;

/// Convenience re-exports for common usage.
pub mod prelude {
    pub use crate::entities::*;
    pub use crate::error::StoreError;
    pub use crate::store::RunStore;
    pub use crate::user_store::UserStore;

    #[cfg(feature = "store-memory")]
    pub use crate::memory::InMemoryStore;

    #[cfg(feature = "store-postgres")]
    pub use crate::postgres::PostgresStore;
}
