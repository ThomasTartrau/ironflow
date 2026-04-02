//! Auth routes — one module per endpoint.

pub mod me;
pub mod refresh;
pub mod sign_in;
pub mod sign_out;
#[cfg(feature = "sign-up")]
pub mod sign_up;
