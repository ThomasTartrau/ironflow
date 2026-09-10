//! Secret management MCP tools (admin only).

use rust_mcp_sdk::schema::schema_utils::CallToolError;

use crate::error::McpError;

pub mod create;
pub mod delete;
pub mod list;
pub mod rotate;
pub mod update;

pub use create::CreateSecretTool;
pub use delete::DeleteSecretTool;
pub use list::ListSecretsTool;
pub use rotate::RotateSecretKeyTool;
pub use update::UpdateSecretTool;

pub(crate) fn validate_secret_key(key: &str) -> Result<(), CallToolError> {
    if key.is_empty() || key.contains("..") || key.contains('\\') {
        return Err(CallToolError::new(McpError::Validation(
            "invalid secret key: must not be empty or contain '..' or '\\'".to_string(),
        )));
    }
    Ok(())
}
