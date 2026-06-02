//! # ironflow-sdk
//!
//! Type-safe Rust SDK for the [Ironflow](https://gitlab.com/ThomasTartrau/ironflow)
//! workflow engine API. Generated from the OpenAPI specification via
//! [progenitor](https://github.com/oxidecomputer/progenitor), with ergonomic
//! wrappers for authentication and SSE event streaming.
//!
//! # Quick start
//!
//! ```no_run
//! use ironflow_sdk::IronflowClient;
//!
//! # async fn example() -> Result<(), ironflow_sdk::Error> {
//! let client = IronflowClient::new("https://ironflow.example.com", "my-api-key");
//!
//! let runs = client.list_runs().await?;
//! for run in &runs.data {
//!     # let _ = run;
//! }
//! # Ok(())
//! # }
//! ```
//!
//! # Authentication
//!
//! The SDK uses Bearer token authentication. Pass your API key when creating
//! the client -- it is attached to every request automatically.
//!
//! # SSE Event Streaming
//!
//! ```no_run
//! use ironflow_sdk::IronflowClient;
//! use futures_util::StreamExt;
//!
//! # async fn example() -> Result<(), ironflow_sdk::Error> {
//! let client = IronflowClient::new("https://ironflow.example.com", "my-api-key");
//! let mut stream = client.events(None, None).await?;
//!
//! while let Some(event) = stream.next().await {
//!     match event {
//!         Ok(ev) => { /* handle event */ }
//!         Err(e) => { /* handle error */ }
//!     }
//! }
//! # Ok(())
//! # }
//! ```

pub mod client;
pub mod error;
#[doc(hidden)]
#[allow(unused_imports, clippy::all)]
mod generated {
    include!(concat!(env!("OUT_DIR"), "/codegen.rs"));
}
pub mod sse;

pub use client::IronflowClient;
pub use error::Error;
pub use generated::types;
