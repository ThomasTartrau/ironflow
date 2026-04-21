//! API entities — DTOs and query parameter types.
//!
//! These types form the public API contract. They map from internal store
//! models and are never exposed directly.

mod auth;
mod create_run;
mod run;
mod secret;
mod stats;
mod step;
mod user;

pub use auth::{MeResponse, SignInRequest, SignUpRequest};
pub use create_run::CreateRunRequest;
pub use run::{ListRunsQuery, RunDetailResponse, RunResponse};
pub use secret::{SecretResponse, SetSecretRequest};
pub use stats::StatsResponse;
pub use step::StepResponse;
pub use user::{CreateUserRequest, UpdateRoleRequest, UserResponse};
