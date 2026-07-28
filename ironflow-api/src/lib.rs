//! # ironflow-api
//!
//! REST API crate for the **ironflow** workflow engine. Provides endpoints for
//! querying workflow runs, managing their lifecycle, and viewing aggregate statistics.
//!
//! # Architecture
//!
//! - `entities/` — DTOs and query parameter types (public API contract)
//! - `routes/` — One file per route handler
//! - `error.rs` — Typed API errors mapped to HTTP status codes
//! - `response.rs` — Standard response envelope
//! - `state.rs` — Shared application state
//!
//! # API Endpoints
//!
//! ## Health check
//! - `GET /api/v1/health-check` — Liveness probe, always returns 200 OK
//!
//! ## Runs
//! - `GET /api/v1/runs` — List runs with optional filtering and pagination
//! - `POST /api/v1/runs` — Trigger a workflow
//! - `GET /api/v1/runs/:id` — Get run details and steps
//! - `POST /api/v1/runs/:id/cancel` — Cancel a pending or running run
//! - `POST /api/v1/runs/:id/retry` — Retry a failed run (creates new run)
//!
//! ## Workflows
//! - `GET /api/v1/workflows` — List registered workflows
//!
//! ## Statistics
//! - `GET /api/v1/stats` — Aggregate statistics (total runs, success rate, cost, etc.)
//!
//! ## Events (SSE)
//! - `GET /api/v1/events` — Server-Sent Events stream for real-time updates
//!
//! # Quick start
//!
//! ```no_run
//! use ironflow_api::prelude::*;
//! use ironflow_api::routes::{RouterConfig, create_router};
//! use ironflow_store::prelude::*;
//! use ironflow_engine::engine::Engine;
//! use ironflow_core::providers::claude::ClaudeCodeProvider;
//! use ironflow_auth::jwt::JwtConfig;
//! use std::sync::Arc;
//!
//! # async fn example() {
//! let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
//! let provider = Arc::new(ClaudeCodeProvider::new());
//! let engine = Arc::new(Engine::new(store.clone(), provider));
//! let jwt_config = Arc::new(JwtConfig {
//!     secret: "your-secret-key".to_string(),
//!     access_token_ttl_secs: 900,
//!     refresh_token_ttl_secs: 604800,
//!     cookie_domain: None,
//!     cookie_secure: false,
//! });
//! let broadcaster = ironflow_api::sse::SseBroadcaster::new();
//! let state = AppState::new(store, engine, jwt_config, "token".to_string(), broadcaster.sender());
//! let app = create_router(state, RouterConfig::default());
//!
//! let listener = tokio::net::TcpListener::bind("127.0.0.1:3000")
//!     .await
//!     .unwrap();
//! axum::serve(listener, app).await.unwrap();
//! # }
//! ```

pub mod config;
#[cfg(feature = "dashboard")]
pub mod dashboard;
pub mod entities;
pub mod error;
pub mod middleware;
#[cfg(feature = "openapi")]
pub mod openapi;
pub mod rate_limit;
pub mod reaper;
pub mod response;
pub mod routes;
pub mod sse;
pub mod state;

/// Convenience re-exports for common API usage.
pub mod prelude {
    pub use crate::error::ApiError;
    pub use crate::response::{ApiMeta, ApiResponse, ok, ok_paged};
    pub use crate::routes::{RouterConfig, create_router};
    pub use crate::state::AppState;
    pub use ironflow_store::store::Store;
}

pub use routes::{RouterConfig, create_router};
pub use state::AppState;
