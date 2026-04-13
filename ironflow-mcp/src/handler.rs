//! MCP server handler implementation.

use std::sync::Arc;

use async_trait::async_trait;
use rust_mcp_sdk::McpServer;
use rust_mcp_sdk::mcp_server::ServerHandler;
use rust_mcp_sdk::schema::{
    CallToolRequestParams, CallToolResult, ListToolsResult, PaginatedRequestParams, RpcError,
    schema_utils::CallToolError,
};

use crate::client::ApiClient;
use crate::tools::IronflowTools;

/// MCP server handler that dispatches tool calls to the Ironflow API.
pub struct IronflowHandler {
    client: Arc<ApiClient>,
}

impl IronflowHandler {
    /// Create a new handler with the given API client.
    pub fn new(client: Arc<ApiClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl ServerHandler for IronflowHandler {
    async fn handle_list_tools_request(
        &self,
        _params: Option<PaginatedRequestParams>,
        _runtime: Arc<dyn McpServer>,
    ) -> Result<ListToolsResult, RpcError> {
        Ok(ListToolsResult {
            tools: IronflowTools::tools(),
            meta: None,
            next_cursor: None,
        })
    }

    async fn handle_call_tool_request(
        &self,
        params: CallToolRequestParams,
        _runtime: Arc<dyn McpServer>,
    ) -> Result<CallToolResult, CallToolError> {
        let tool = IronflowTools::try_from(params).map_err(CallToolError::new)?;

        match tool {
            IronflowTools::ListWorkflowsTool(t) => t.run(&self.client).await,
            IronflowTools::GetWorkflowTool(t) => t.run(&self.client).await,
            IronflowTools::CreateRunTool(t) => t.run(&self.client).await,
            IronflowTools::ListRunsTool(t) => t.run(&self.client).await,
            IronflowTools::GetRunTool(t) => t.run(&self.client).await,
            IronflowTools::CancelRunTool(t) => t.run(&self.client).await,
            IronflowTools::ApproveRunTool(t) => t.run(&self.client).await,
            IronflowTools::RejectRunTool(t) => t.run(&self.client).await,
            IronflowTools::RetryRunTool(t) => t.run(&self.client).await,
            IronflowTools::GetStatsTool(t) => t.run(&self.client).await,
        }
    }
}
