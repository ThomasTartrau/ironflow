//! `download_artifact` MCP tool.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::CallToolResult;
use rust_mcp_sdk::schema::schema_utils::CallToolError;

use crate::client::ApiClient;
use crate::error::McpError;

/// Download an artifact produced by a workflow step.
#[mcp_tool(
    name = "download_artifact",
    description = "Download an artifact produced by a step in a workflow run. Returns the artifact content as text. The artifact must belong to the specified run and step."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct DownloadArtifactTool {
    /// The run ID (UUID).
    pub run_id: String,
    /// The step ID (UUID).
    pub step_id: String,
    /// The artifact name (e.g. `report.html`, `output.json`).
    pub name: String,
}

impl DownloadArtifactTool {
    /// Execute the tool against the Ironflow API.
    pub async fn run(&self, client: &ApiClient) -> Result<CallToolResult, CallToolError> {
        validate_path_segment(&self.run_id, "run_id")?;
        validate_path_segment(&self.step_id, "step_id")?;
        validate_artifact_name(&self.name)?;

        let path = format!(
            "/runs/{}/steps/{}/artifacts/{}",
            self.run_id, self.step_id, self.name,
        );

        let (bytes, _content_type) = client.get_bytes(&path).await.map_err(CallToolError::new)?;

        let text = String::from_utf8(bytes).unwrap_or_else(|e| {
            let raw = e.into_bytes();
            format!(
                "[base64-encoded binary artifact]\n{}",
                STANDARD.encode(&raw)
            )
        });

        Ok(CallToolResult::text_content(vec![text.into()]))
    }
}

fn validate_path_segment(value: &str, field: &str) -> Result<(), CallToolError> {
    if value.is_empty() || value.contains('/') || value.contains('\\') || value.contains("..") {
        return Err(CallToolError::new(McpError::Validation(format!(
            "invalid {field}: must not be empty or contain '/', '\\\\', '..'"
        ))));
    }
    Ok(())
}

fn validate_artifact_name(name: &str) -> Result<(), CallToolError> {
    if name.is_empty() || name.contains("..") || name.contains('/') || name.contains('\\') {
        return Err(CallToolError::new(McpError::Validation(
            "invalid artifact name: must not be empty or contain '..', '/', '\\'".to_string(),
        )));
    }
    Ok(())
}
