//! # ironflow-worker
//!
//! HTTP-based background worker for ironflow. Polls the API server for
//! pending workflow runs, executes them locally, and reports results back.
//!
//! The worker communicates with the API via `/api/v1/internal/*` routes,
//! authenticated with a static `WORKER_TOKEN`.
//!
//! # Quick start
//!
//! ```no_run
//! use std::sync::Arc;
//! use std::time::Duration;
//! use ironflow_worker::WorkerBuilder;
//! use ironflow_core::providers::claude::ClaudeCodeProvider;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let worker = WorkerBuilder::new("http://localhost:3000", "my-worker-token")
//!     .provider(Arc::new(ClaudeCodeProvider::new()))
//!     .concurrency(4)
//!     .poll_interval(Duration::from_secs(2))
//!     .build()?;
//!
//! worker.run().await?;
//! # Ok(())
//! # }
//! ```

mod api_store;
pub mod error;
mod log_pusher;
pub mod worker;

pub use error::WorkerError;
pub use worker::{Worker, WorkerBuilder};
