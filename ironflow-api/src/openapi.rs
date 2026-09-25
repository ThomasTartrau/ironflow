//! OpenAPI/Swagger documentation for ironflow-api.

use crate::entities::{
    ApprovalDelegationResponse, ArtifactResponse, ChangePasswordRequest,
    CreateApprovalDelegationRequest, CreateRunRequest, CreateScheduleRequest, CreateUserRequest,
    CreatedBy, CreatedByKind, KeyVersionsResponse, ListApprovalDelegationsQuery, ListRunsQuery,
    MeResponse, RotateSecretsRequest, RotateSecretsResponse, RunDetailResponse, RunResponse,
    ScheduleResponse, SecretResponse, SetSecretRequest, SignInRequest, StatsHistoryBucketResponse,
    StatsHistoryResponse, StatsResponse, StepResponse, UpdateRoleRequest, UpdateScheduleRequest,
    UpdateUserGroupsRequest, UserGroupsResponse, UserResponse,
};
use crate::routes::api_keys::available_scopes::ScopeEntry;
use crate::routes::api_keys::create::{CreateApiKeyRequest, CreateApiKeyResponse};
use crate::routes::api_keys::list::ApiKeyResponse;
use crate::routes::audit_logs::ListAuditLogsQuery;
use crate::routes::events::EventKind;
use crate::routes::get_run_logs::{GetRunLogsQuery, LogCursorMeta};
use crate::routes::get_workflow::{SubWorkflowDetail, WorkflowDetailResponse};
use crate::routes::list_workflows::{ListWorkflowsQuery, WorkflowSummary};
use crate::routes::plan_workflow::{
    ConditionResponse, ExecutionPlanResponse, PlanWorkflowRequest, PlannedStepResponse,
};
use crate::routes::secrets::update::UpdateSecretRequest;
use crate::routes::users::list::ListUsersQuery;
use crate::routes::{
    api_keys, approval_delegations, approve_run, audit_logs, auth, cancel_run, create_run,
    download_artifact, get_run, get_run_logs, get_stats, get_stats_history, get_workflow,
    health_check, list_runs, list_workflows, plan_workflow, replay_run, retry_run, run_events,
    schedules, secrets, users,
};
use ironflow_engine::notify::{
    ApprovalEscalatedEvent, ApprovalGrantedEvent, ApprovalRejectedEvent, ApprovalRequestedEvent,
    Event, LogLineEvent, RetryForcedEvent, RunBudgetExceededEvent, RunCreatedEvent, RunFailedEvent,
    RunStatusChangedEvent, StepCompletedEvent, StepFailedEvent, UserSignedInEvent,
    UserSignedOutEvent, UserSignedUpEvent, WorkflowAgentStepTokensUsedEvent,
    WorkflowApprovalRequiredEvent, WorkflowEvent, WorkflowStepCompletedEvent,
    WorkflowStepFailedEvent, WorkflowStepStartedEvent,
};
use ironflow_store::entities::{
    ApprovalRequirement, AuditLogEntry, LogEntry, LogStream, StepApproval,
};
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::{Modify, OpenApi};

/// Adds the `Bearer` security scheme to the OpenAPI spec.
struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_default();
        components.add_security_scheme(
            "Bearer",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .bearer_format("JWT")
                    .build(),
            ),
        );
    }
}

#[cfg(feature = "sign-up")]
mod with_signup {
    use super::*;
    use crate::entities::SignUpRequest;

    /// OpenAPI documentation for ironflow REST API (with sign-up).
    #[derive(OpenApi)]
    #[openapi(
        info(
            title = "Ironflow REST API",
            description = "REST API for the ironflow workflow engine",
            version = "1.0.0"
        ),
        modifiers(&SecurityAddon),
        paths(
            health_check::health_check,
            list_runs::list_runs,
            create_run::create_run,
            get_run::get_run,
            cancel_run::cancel_run,
            approve_run::approve_run,
            approve_run::reject_run,
            retry_run::retry_run,
            replay_run::replay_run,
            download_artifact::download_artifact,
            list_workflows::list_workflows,
            get_workflow::get_workflow,
            plan_workflow::plan_workflow,
            get_stats::get_stats,
            get_stats_history::get_stats_history,
            auth::sign_up::sign_up,
            auth::sign_in::sign_in,
            auth::refresh::refresh,
            auth::sign_out::sign_out,
            auth::me::me,
            api_keys::list::list_api_keys,
            api_keys::create::create_api_key,
            api_keys::available_scopes::available_scopes,
            api_keys::delete::delete_api_key,
            users::list::list_users,
            users::create::create_user,
            users::delete::delete_user,
            users::update_role::update_role,
            users::groups::get_user_groups,
            users::groups::update_user_groups,
            secrets::create::create_secret,
            secrets::list::list_secrets,
            secrets::update::update_secret,
            secrets::delete::delete_secret,
            secrets::rotate::rotate_secrets,
            secrets::key_versions::secret_key_versions,
            audit_logs::list_audit_logs,
            get_run_logs::get_run_logs,
            run_events::run_events,
            auth::change_password::change_password,
            schedules::create::create_schedule,
            schedules::list::list_schedules,
            schedules::get::get_schedule,
            schedules::delete::delete_schedule,
            schedules::pause_resume::pause_schedule,
            schedules::pause_resume::resume_schedule,
            schedules::trigger::trigger_schedule,
            approval_delegations::create::create_approval_delegation,
            approval_delegations::list::list_approval_delegations,
            approval_delegations::delete::delete_approval_delegation,
        ),
        components(
            schemas(
                RunResponse,
                RunDetailResponse,
                StepResponse,
                ArtifactResponse,
                CreatedBy,
                CreatedByKind,
                CreateRunRequest,
                StatsResponse,
                StatsHistoryResponse,
                StatsHistoryBucketResponse,
                MeResponse,
                SignInRequest,
                SignUpRequest,
                CreateUserRequest,
                UserResponse,
                UpdateRoleRequest,
                UpdateUserGroupsRequest,
                UserGroupsResponse,
                ListWorkflowsQuery,
                WorkflowSummary,
                WorkflowDetailResponse,
                SubWorkflowDetail,
                PlanWorkflowRequest,
                ExecutionPlanResponse,
                PlannedStepResponse,
                ConditionResponse,
                ListRunsQuery,
                ApiKeyResponse,
                CreateApiKeyRequest,
                CreateApiKeyResponse,
                ScopeEntry,
                ListUsersQuery,
                SecretResponse,
                SetSecretRequest,
                UpdateSecretRequest,
                RotateSecretsRequest,
                RotateSecretsResponse,
                KeyVersionsResponse,
                EventKind,
                Event,
                RunCreatedEvent,
                RunStatusChangedEvent,
                RunFailedEvent,
                RunBudgetExceededEvent,
                RetryForcedEvent,
                StepCompletedEvent,
                StepFailedEvent,
                ApprovalRequestedEvent,
                ApprovalGrantedEvent,
                ApprovalRejectedEvent,
                ApprovalRequirement,
                StepApproval,
                ApprovalEscalatedEvent,
                LogLineEvent,
                UserSignedInEvent,
                UserSignedUpEvent,
                UserSignedOutEvent,
                WorkflowEvent,
                WorkflowStepStartedEvent,
                WorkflowStepCompletedEvent,
                WorkflowStepFailedEvent,
                WorkflowApprovalRequiredEvent,
                WorkflowAgentStepTokensUsedEvent,
                AuditLogEntry,
                ListAuditLogsQuery,
                LogEntry,
                LogStream,
                GetRunLogsQuery,
                LogCursorMeta,
                ChangePasswordRequest,
                ScheduleResponse,
                CreateScheduleRequest,
                UpdateScheduleRequest,
                ApprovalDelegationResponse,
                CreateApprovalDelegationRequest,
                ListApprovalDelegationsQuery,
            )
        ),
        tags(
            (name = "health", description = "Health check endpoints"),
            (name = "runs", description = "Workflow run management"),
            (name = "workflows", description = "Workflow definitions"),
            (name = "stats", description = "Aggregated statistics"),
            (name = "auth", description = "Authentication and authorization"),
            (name = "api-keys", description = "API key management"),
            (name = "users", description = "User management (admin only)"),
            (name = "secrets", description = "Encrypted secret management (admin only)"),
            (name = "audit", description = "Audit log (admin only)"),
            (name = "logs", description = "Run/step log persistence and retrieval"),
            (name = "schedules", description = "Schedule management"),
            (name = "approval-delegations", description = "Approval delegation for absent approvers"),
        )
    )]
    pub struct ApiDoc;
}

#[cfg(not(feature = "sign-up"))]
mod without_signup {
    use super::*;

    /// OpenAPI documentation for ironflow REST API (without sign-up).
    #[derive(OpenApi)]
    #[openapi(
        info(
            title = "Ironflow REST API",
            description = "REST API for the ironflow workflow engine",
            version = "1.0.0"
        ),
        modifiers(&SecurityAddon),
        paths(
            health_check::health_check,
            list_runs::list_runs,
            create_run::create_run,
            get_run::get_run,
            cancel_run::cancel_run,
            approve_run::approve_run,
            approve_run::reject_run,
            retry_run::retry_run,
            replay_run::replay_run,
            download_artifact::download_artifact,
            list_workflows::list_workflows,
            get_workflow::get_workflow,
            plan_workflow::plan_workflow,
            get_stats::get_stats,
            get_stats_history::get_stats_history,
            auth::sign_in::sign_in,
            auth::refresh::refresh,
            auth::sign_out::sign_out,
            auth::me::me,
            api_keys::list::list_api_keys,
            api_keys::create::create_api_key,
            api_keys::available_scopes::available_scopes,
            api_keys::delete::delete_api_key,
            users::list::list_users,
            users::create::create_user,
            users::delete::delete_user,
            users::update_role::update_role,
            users::groups::get_user_groups,
            users::groups::update_user_groups,
            secrets::create::create_secret,
            secrets::list::list_secrets,
            secrets::update::update_secret,
            secrets::delete::delete_secret,
            secrets::rotate::rotate_secrets,
            secrets::key_versions::secret_key_versions,
            audit_logs::list_audit_logs,
            get_run_logs::get_run_logs,
            run_events::run_events,
            auth::change_password::change_password,
            schedules::create::create_schedule,
            schedules::list::list_schedules,
            schedules::get::get_schedule,
            schedules::delete::delete_schedule,
            schedules::pause_resume::pause_schedule,
            schedules::pause_resume::resume_schedule,
            schedules::trigger::trigger_schedule,
            approval_delegations::create::create_approval_delegation,
            approval_delegations::list::list_approval_delegations,
            approval_delegations::delete::delete_approval_delegation,
        ),
        components(
            schemas(
                RunResponse,
                RunDetailResponse,
                StepResponse,
                ArtifactResponse,
                CreatedBy,
                CreatedByKind,
                CreateRunRequest,
                StatsResponse,
                StatsHistoryResponse,
                StatsHistoryBucketResponse,
                MeResponse,
                SignInRequest,
                CreateUserRequest,
                UserResponse,
                UpdateRoleRequest,
                UpdateUserGroupsRequest,
                UserGroupsResponse,
                ListWorkflowsQuery,
                WorkflowSummary,
                WorkflowDetailResponse,
                SubWorkflowDetail,
                PlanWorkflowRequest,
                ExecutionPlanResponse,
                PlannedStepResponse,
                ConditionResponse,
                ListRunsQuery,
                ApiKeyResponse,
                CreateApiKeyRequest,
                CreateApiKeyResponse,
                ScopeEntry,
                ListUsersQuery,
                SecretResponse,
                SetSecretRequest,
                UpdateSecretRequest,
                RotateSecretsRequest,
                RotateSecretsResponse,
                KeyVersionsResponse,
                EventKind,
                Event,
                RunCreatedEvent,
                RunStatusChangedEvent,
                RunFailedEvent,
                RunBudgetExceededEvent,
                RetryForcedEvent,
                StepCompletedEvent,
                StepFailedEvent,
                ApprovalRequestedEvent,
                ApprovalGrantedEvent,
                ApprovalRejectedEvent,
                ApprovalRequirement,
                StepApproval,
                LogLineEvent,
                UserSignedInEvent,
                UserSignedUpEvent,
                UserSignedOutEvent,
                WorkflowEvent,
                WorkflowStepStartedEvent,
                WorkflowStepCompletedEvent,
                WorkflowStepFailedEvent,
                WorkflowApprovalRequiredEvent,
                WorkflowAgentStepTokensUsedEvent,
                AuditLogEntry,
                ListAuditLogsQuery,
                LogEntry,
                LogStream,
                GetRunLogsQuery,
                LogCursorMeta,
                ChangePasswordRequest,
                ScheduleResponse,
                CreateScheduleRequest,
                UpdateScheduleRequest,
                ApprovalDelegationResponse,
                CreateApprovalDelegationRequest,
                ListApprovalDelegationsQuery,
            )
        ),
        tags(
            (name = "health", description = "Health check endpoints"),
            (name = "runs", description = "Workflow run management"),
            (name = "workflows", description = "Workflow definitions"),
            (name = "stats", description = "Aggregated statistics"),
            (name = "auth", description = "Authentication and authorization"),
            (name = "api-keys", description = "API key management"),
            (name = "users", description = "User management (admin only)"),
            (name = "secrets", description = "Encrypted secret management (admin only)"),
            (name = "audit", description = "Audit log (admin only)"),
            (name = "logs", description = "Run/step log persistence and retrieval"),
            (name = "schedules", description = "Schedule management"),
            (name = "approval-delegations", description = "Approval delegation for absent approvers"),
        )
    )]
    pub struct ApiDoc;
}

#[cfg(feature = "sign-up")]
pub use with_signup::ApiDoc;

#[cfg(not(feature = "sign-up"))]
pub use without_signup::ApiDoc;
