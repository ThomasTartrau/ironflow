//! # ironflow-runtime
//!
//! The daemon/server layer for **ironflow**, providing webhook HTTP endpoints
//! and trigger sources on top of [`ironflow_core`] operations.
//!
//! This crate exposes a [`runtime::Runtime`] builder that lets you declaratively register
//! webhook routes (with pluggable authentication), then start a full
//! [Axum](https://docs.rs/axum) HTTP server via [`runtime::Runtime::serve`] with
//! graceful shutdown support.
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
//! - [`trigger`] - Pluggable trigger sources (`EventTrigger`,
//!   `NatsTrigger` behind `trigger-nats` feature).

pub mod error;
pub mod runtime;
pub mod trigger;
pub mod webhook;

/// Convenience re-exports for common usage.
///
/// # Contents
///
/// - [`Runtime`](crate::runtime::Runtime) - The server builder.
/// - [`WebhookAuth`](crate::webhook::WebhookAuth) - Webhook authentication configuration.
pub mod prelude {
    pub use crate::error::RuntimeError;
    pub use crate::runtime::{Runtime, WebhookContext};
    pub use crate::trigger::{Trigger, TriggerEvent, TriggerSink};
    pub use crate::webhook::WebhookAuth;
}
