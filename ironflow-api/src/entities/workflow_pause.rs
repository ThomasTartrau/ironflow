//! Workflow pause response DTO.

use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use ironflow_store::entities::WorkflowPause;

/// Response of `POST /api/v1/workflows/:name/pause` and
/// `POST /api/v1/workflows/:name/resume`.
///
/// # Examples
///
/// ```
/// use ironflow_api::entities::WorkflowPauseResponse;
///
/// let response = WorkflowPauseResponse::resumed("deploy");
/// assert!(response.paused_at.is_none());
/// ```
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Serialize)]
pub struct WorkflowPauseResponse {
    /// Workflow name.
    pub workflow_name: String,
    /// When the workflow was paused: its queued runs are not picked up until
    /// it is resumed. Omitted when the workflow is not paused.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paused_at: Option<DateTime<Utc>>,
    /// User who paused the workflow, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paused_by: Option<Uuid>,
}

impl WorkflowPauseResponse {
    /// Response for a workflow that is no longer paused.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_api::entities::WorkflowPauseResponse;
    ///
    /// let response = WorkflowPauseResponse::resumed("deploy");
    /// assert_eq!(response.workflow_name, "deploy");
    /// assert!(response.paused_at.is_none());
    /// ```
    pub fn resumed(workflow_name: &str) -> Self {
        Self {
            workflow_name: workflow_name.to_string(),
            paused_at: None,
            paused_by: None,
        }
    }
}

impl From<WorkflowPause> for WorkflowPauseResponse {
    fn from(pause: WorkflowPause) -> Self {
        Self {
            workflow_name: pause.workflow_name,
            paused_at: Some(pause.paused_at),
            paused_by: pause.paused_by,
        }
    }
}
