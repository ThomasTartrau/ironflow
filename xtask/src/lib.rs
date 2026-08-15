//! ironflow developer tasks as a library.
//!
//! The [`seed`] module is reusable: the example server calls
//! [`seed::seed_store`] at startup when `IRONFLOW_SEED=true`.

pub mod seed;
