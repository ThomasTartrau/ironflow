//! API entities — DTOs and query parameter types.
//!
//! These types form the public API contract. They map from internal store
//! models and are never exposed directly.

mod approval_delegation;
mod artifact;
mod auth;
mod create_run;
mod created_by;
pub mod lease;
mod provider_account;
mod run;
mod schedule;
mod secret;
mod signal;
mod stats;
mod step;
mod user;

pub use approval_delegation::{
    ApprovalDelegationResponse, CreateApprovalDelegationRequest, ListApprovalDelegationsQuery,
};
pub use artifact::ArtifactResponse;
pub use auth::{ChangePasswordRequest, MeResponse, SignInRequest, SignUpRequest};
pub use create_run::{CreateRunRequest, IdempotencyKeyError, validate_idempotency_key};
pub use created_by::{CreatedBy, CreatedByKind};
pub use lease::{
    DEFAULT_LEASE_TTL_SECS, MAX_LEASE_TTL_SECS, RenewLeaseRequest, RenewLeaseResponse,
    validate_lease_ttl,
};
pub use provider_account::{
    AccountFormFieldResponse, AccountKindResponse, AccountState, AccountTestResult,
    AccountUsagePointResponse, AccountWindowResponse, CreateProviderAccountRequest,
    ListProviderAccountsQuery, ProviderAccountResponse, ProviderAccountTestResponse,
    ProviderAccountUsageResponse, UpdateProviderAccountRequest, UsageQuery,
};
pub use run::{ListRunsQuery, RunDetailResponse, RunResponse};
pub use schedule::{CreateScheduleRequest, ScheduleResponse, UpdateScheduleRequest};
pub use secret::{
    KeyVersionsResponse, RotateSecretsRequest, RotateSecretsResponse, SecretResponse,
    SetSecretRequest,
};
pub use signal::{
    ListSignalsQuery, RejectedRunResponse, ResumedRunResponse, SendSignalRequest,
    SignalDeliveryResponse, SignalResponse,
};
pub use stats::{
    StatsHistoryBucketResponse, StatsHistoryQuery, StatsHistoryResponse, StatsResponse,
};
pub use step::{StepAccountResponse, StepResponse};
pub use user::{
    CreateUserRequest, UpdateRoleRequest, UpdateUserGroupsRequest, UserGroupsResponse, UserResponse,
};
