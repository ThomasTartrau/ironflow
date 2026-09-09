//! Ingester lifecycle operations.
//!
//! These operations control the Mimir ingester's lifecycle: flushing data,
//! managing graceful shutdown, partition downscaling, and ring/tenant queries.

mod lifecycle;
mod query;

pub use lifecycle::{
    CancelPartitionDownscale, CancelShutdown, Flush, PreparePartitionDownscale, PrepareShutdown,
    Shutdown,
};
pub use query::{GetIngesterRing, GetIngesterTenants};
