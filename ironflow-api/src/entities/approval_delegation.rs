//! Approval delegation request and response DTOs.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use validator::Validate;

use ironflow_store::entities::ApprovalDelegation;

/// Approval delegation response.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Serialize)]
pub struct ApprovalDelegationResponse {
    /// Delegation ID.
    pub id: Uuid,
    /// User handing over their approval power.
    pub from_user_id: Uuid,
    /// User receiving the approval power.
    pub to_user_id: Uuid,
    /// Start of the validity window (inclusive).
    pub valid_from: DateTime<Utc>,
    /// End of the validity window (exclusive).
    pub valid_until: DateTime<Utc>,
    /// Glob on the workflow name. `None` means every workflow.
    pub workflow_filter: Option<String>,
    /// When the delegation was created.
    pub created_at: DateTime<Utc>,
}

impl From<ApprovalDelegation> for ApprovalDelegationResponse {
    fn from(d: ApprovalDelegation) -> Self {
        Self {
            id: d.id,
            from_user_id: d.from_user_id,
            to_user_id: d.to_user_id,
            valid_from: d.valid_from,
            valid_until: d.valid_until,
            workflow_filter: d.workflow_filter,
            created_at: d.created_at,
        }
    }
}

/// Create delegation request body.
///
/// The delegator is always the authenticated caller: a delegation can only be
/// granted by the person handing over their own approval power.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Deserialize, Validate)]
pub struct CreateApprovalDelegationRequest {
    /// User receiving the delegated approval power.
    pub to_user_id: Uuid,
    /// Start of the window. Defaults to now when omitted.
    pub valid_from: Option<DateTime<Utc>>,
    /// End of the window (exclusive).
    pub valid_until: DateTime<Utc>,
    /// Optional glob on the workflow name, e.g. `"deploy-*"`. `None` = all workflows.
    pub workflow_filter: Option<String>,
}

/// Query parameters for listing delegations.
///
/// Both filters are honoured for an admin only; a non-admin always sees exactly
/// the delegations they granted or received.
#[cfg_attr(feature = "openapi", derive(utoipa::IntoParams, utoipa::ToSchema))]
#[derive(Debug, Deserialize)]
pub struct ListApprovalDelegationsQuery {
    /// Only delegations granted by this user.
    pub from_user_id: Option<Uuid>,
    /// Only delegations received by this user.
    pub to_user_id: Option<Uuid>,
}
