//! Compactor operations.
//!
//! These operations manage the compactor: ring status, block uploads,
//! and tenant listing.

mod query;
mod upload;

pub use query::{GetRing, GetTenants};
pub use upload::{FinishBlockUpload, StartBlockUpload, UploadBlockFile};
