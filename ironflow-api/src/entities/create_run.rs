//! Request type for triggering a workflow.

use serde::Deserialize;
use serde_json::Value;

/// Request to trigger a workflow.
///
/// # Examples
///
/// ```
/// use ironflow_api::entities::CreateRunRequest;
/// use serde_json::json;
///
/// let req = CreateRunRequest {
///     workflow: "deploy".to_string(),
///     payload: Some(json!({"env": "prod"})),
/// };
/// assert_eq!(req.workflow, "deploy");
/// ```
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Deserialize)]
pub struct CreateRunRequest {
    /// The workflow name to trigger.
    pub workflow: String,
    /// Optional input payload for the workflow.
    pub payload: Option<Value>,
}
