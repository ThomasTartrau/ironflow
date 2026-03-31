//! API entities — DTOs and query parameter types.
//!
//! These types form the public API contract. They map from internal store
//! models and are never exposed directly.

mod auth;
mod create_run;
mod run;
mod stats;
mod step;

pub use auth::{MeResponse, SignInRequest, SignUpRequest};
pub use create_run::CreateRunRequest;
pub use run::{ListRunsQuery, RunDetailResponse, RunResponse};
pub use stats::StatsResponse;
pub use step::StepResponse;
