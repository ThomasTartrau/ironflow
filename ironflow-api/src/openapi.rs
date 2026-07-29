//! OpenAPI/Swagger documentation for ironflow-api.

use crate::entities::{
    ArtifactResponse, CreateRunRequest, CreateUserRequest, CreatedBy, CreatedByKind, ListRunsQuery,
    MeResponse, RunDetailResponse, RunResponse, SecretResponse, SetSecretRequest, SignInRequest,
    StatsResponse, StepResponse, UpdateRoleRequest, UserResponse,
};
use crate::routes::api_keys::available_scopes::ScopeEntry;
use crate::routes::api_keys::create::{CreateApiKeyRequest, CreateApiKeyResponse};
use crate::routes::api_keys::list::ApiKeyResponse;
use crate::routes::audit_logs::ListAuditLogsQuery;
use crate::routes::events::EventKind;
use crate::routes::get_workflow::{SubWorkflowDetail, WorkflowDetailResponse};
use crate::routes::list_workflows::{ListWorkflowsQuery, WorkflowSummary};
use crate::routes::secrets::update::UpdateSecretRequest;
use crate::routes::users::list::ListUsersQuery;
use crate::routes::{
    api_keys, approve_run, audit_logs, auth, cancel_run, create_run, download_artifact, get_run,
    get_stats, get_workflow, health_check, list_runs, list_workflows, retry_run, secrets, users,
};
use ironflow_engine::notify::Event;
use ironflow_store::entities::AuditLogEntry;
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
            download_artifact::download_artifact,
            list_workflows::list_workflows,
            get_workflow::get_workflow,
            get_stats::get_stats,
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
            secrets::create::create_secret,
            secrets::list::list_secrets,
            secrets::update::update_secret,
            secrets::delete::delete_secret,
            audit_logs::list_audit_logs,
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
                MeResponse,
                SignInRequest,
                SignUpRequest,
                CreateUserRequest,
                UserResponse,
                UpdateRoleRequest,
                ListWorkflowsQuery,
                WorkflowSummary,
                WorkflowDetailResponse,
                SubWorkflowDetail,
                ListRunsQuery,
                ApiKeyResponse,
                CreateApiKeyRequest,
                CreateApiKeyResponse,
                ScopeEntry,
                ListUsersQuery,
                SecretResponse,
                SetSecretRequest,
                UpdateSecretRequest,
                EventKind,
                Event,
                AuditLogEntry,
                ListAuditLogsQuery,
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
            download_artifact::download_artifact,
            list_workflows::list_workflows,
            get_workflow::get_workflow,
            get_stats::get_stats,
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
            secrets::create::create_secret,
            secrets::list::list_secrets,
            secrets::update::update_secret,
            secrets::delete::delete_secret,
            audit_logs::list_audit_logs,
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
                MeResponse,
                SignInRequest,
                CreateUserRequest,
                UserResponse,
                UpdateRoleRequest,
                ListWorkflowsQuery,
                WorkflowSummary,
                WorkflowDetailResponse,
                SubWorkflowDetail,
                ListRunsQuery,
                ApiKeyResponse,
                CreateApiKeyRequest,
                CreateApiKeyResponse,
                ScopeEntry,
                ListUsersQuery,
                SecretResponse,
                SetSecretRequest,
                UpdateSecretRequest,
                EventKind,
                Event,
                AuditLogEntry,
                ListAuditLogsQuery,
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
        )
    )]
    pub struct ApiDoc;
}

#[cfg(feature = "sign-up")]
pub use with_signup::ApiDoc;

#[cfg(not(feature = "sign-up"))]
pub use without_signup::ApiDoc;
