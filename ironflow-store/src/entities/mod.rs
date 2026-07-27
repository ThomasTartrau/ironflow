//! Store entities — data models split by domain.
//!
//! Each entity lives in its own file. All types are re-exported here
//! for convenient `use ironflow_store::entities::*` access.

pub mod api_key;
pub mod api_key_scope;
mod audit_log;
mod event_kind;
mod fsm_state;
mod page;
mod run;
mod run_status;
mod secret;
mod stats;
mod step;
mod step_dependency;
mod step_kind;
mod step_status;
mod trigger_kind;
mod user;

pub use api_key::{ApiKey, ApiKeyUpdate, NewApiKey};
pub use api_key_scope::ApiKeyScope;
pub use audit_log::{AuditLogEntry, AuditLogFilter, NewAuditLogEntry};
pub use event_kind::EventKind;
pub use fsm_state::FsmState;
pub use page::Page;
pub use run::{
    IDEMPOTENCY_WINDOW, MAX_IDEMPOTENCY_KEY_LEN, NewRun, Run, RunCreation, RunFilter, RunUpdate,
};
pub use run_status::RunStatus;
pub use secret::{Secret, SecretMetadata};
pub use stats::RunStats;
pub use step::{NewStep, Step, StepUpdate};
pub use step_dependency::{NewStepDependency, StepDependency};
pub use step_kind::StepKind;
pub use step_status::StepStatus;
pub use trigger_kind::TriggerKind;
pub use user::{NewUser, User};
