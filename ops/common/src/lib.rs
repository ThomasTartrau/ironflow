//! Shared HTTP client, helpers and test utilities for Ironflow ops crates.
//!
//! This crate provides the generic building blocks that every HTTP-based ops
//! crate (Grafana, Loki, Tempo, and future ones) needs: a multi-auth HTTP
//! client, response-checking helpers, path validation, and JSON utilities.
//!
//! # Architecture
//!
//! - [`HttpApiClient`] is the generic HTTP client with bearer, basic, or no
//!   authentication. Product-specific ops crates wrap it in a thin newtype
//!   (e.g. `TempoClient`, `LokiClient`) that provides product-specific
//!   `from_context` and re-exports the request-building methods.
//! - [`helpers`] provides response checking, path validation, request sending,
//!   and JSON (de)serialization helpers.
//! - The `test-utils` feature exposes `MapSecretResolver` so downstream
//!   crates can test `from_context` with fake secrets.
//!
//! # Quick start
//!
//! ```no_run
//! use ironflow_ops_common::HttpApiClient;
//! use reqwest::Client;
//!
//! let client = HttpApiClient::new("http://api.example.com", Client::new())
//!     .with_bearer_token("my-token");
//! let req = client.get("/api/v1/status");
//! ```
//!
//! # Test utilities
//!
//! Enable the `test-utils` feature to get `MapSecretResolver`:
//!
//! ```toml
//! [dev-dependencies]
//! ironflow-ops-common = { path = "../common", features = ["test-utils"] }
//! ```
//!
//! ```no_run
//! # #[cfg(feature = "test-utils")]
//! use ironflow_ops_common::MapSecretResolver;
//! ```

mod client;
pub mod helpers;

pub use client::Auth;
pub use client::HttpApiClient;

#[cfg(feature = "test-utils")]
mod test_utils;
#[cfg(feature = "test-utils")]
pub use test_utils::MapSecretResolver;
