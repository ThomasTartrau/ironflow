//! [`WorkflowPause`] entity -- a workflow whose pending runs are held back.
//!
//! While a workflow is paused, workers do not pick its `Pending` or
//! `Retrying` runs. Runs already executing are left alone, and new runs are
//! still created: they wait in the queue until the workflow is resumed.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A persisted pause of every queued run of one workflow.
///
/// # Examples
///
/// ```
/// use chrono::Utc;
/// use ironflow_store::entities::WorkflowPause;
///
/// let pause = WorkflowPause {
///     workflow_name: "deploy".to_string(),
///     paused_at: Utc::now(),
///     paused_by: None,
/// };
/// assert_eq!(pause.workflow_name, "deploy");
/// ```
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowPause {
    /// Name of the paused workflow.
    pub workflow_name: String,
    /// When the workflow was first paused. Pausing it again keeps this value.
    pub paused_at: DateTime<Utc>,
    /// User who paused the workflow, when known.
    pub paused_by: Option<Uuid>,
}
