//! Store-gateway operations.
//!
//! These operations query the store-gateway for ring status, tenants,
//! tenant blocks, and shutdown preparation.

mod query;
mod shutdown;

pub use query::{GetRing, GetTenantBlocks, GetTenants};
pub use shutdown::PrepareShutdown;
