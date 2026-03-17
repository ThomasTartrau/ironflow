//! # ironflow-runtime
//!
//! The daemon/server layer for **ironflow**, providing webhook HTTP endpoints and
//! cron scheduling on top of [`ironflow_core`] operations.
//!
//! This crate exposes a [`runtime::Runtime`] builder that lets you declaratively register
//! webhook routes (with pluggable authentication) and cron jobs, then start an
//! [Axum](https://docs.rs/axum) HTTP server with graceful shutdown support.
//!
//! # Quick start
//!
//! ```no_run
//! use ironflow_runtime::prelude::*;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     Runtime::new()
//!         .webhook("/hooks/github", WebhookAuth::github("my-secret"), |payload| async move {
//!             println!("received: {payload}");
//!         })
//!         .cron("0 */5 * * * *", "health-check", || async {
//!             println!("running health check");
//!         })
//!         .serve("0.0.0.0:3000")
//!         .await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! # Modules
//!
//! - [`runtime`] - The [`runtime::Runtime`] builder and HTTP server.
//! - [`webhook`] - Webhook authentication strategies ([`webhook::WebhookAuth`]).
//! - `cron` - Internal cron job representation (crate-private).

pub(crate) mod cron;
pub mod error;
pub mod runtime;
pub mod webhook;

/// Convenience re-exports for common usage.
///
/// # Contents
///
/// - [`Runtime`](crate::runtime::Runtime) - The server builder.
/// - [`WebhookAuth`](crate::webhook::WebhookAuth) - Webhook authentication configuration.
pub mod prelude {
    pub use crate::error::RuntimeError;
    pub use crate::runtime::Runtime;
    pub use crate::webhook::WebhookAuth;
}
