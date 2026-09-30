//! Provider Account MCP tools: list, get, create, update, delete, test, usage.
//!
//! Admin only. No tool response includes an account token.

use rust_mcp_sdk::schema::schema_utils::CallToolError;

use crate::error::McpError;

pub mod create;
pub mod delete;
pub mod get;
pub mod list;
pub mod test;
pub mod update;
pub mod usage;

pub use create::CreateProviderAccountTool;
pub use delete::DeleteProviderAccountTool;
pub use get::GetProviderAccountTool;
pub use list::ListProviderAccountsTool;
pub use test::TestProviderAccountTool;
pub use update::UpdateProviderAccountTool;
pub use usage::ProviderAccountUsageTool;

/// Accept an account name (slug) or UUID, the only safe path segments.
pub(crate) fn validate_account_ref(account: &str) -> Result<(), CallToolError> {
    let valid = !account.is_empty()
        && account.len() <= 64
        && account
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-');
    if valid {
        Ok(())
    } else {
        Err(CallToolError::new(McpError::Validation(
            "invalid account: expected a name or a UUID".to_string(),
        )))
    }
}
