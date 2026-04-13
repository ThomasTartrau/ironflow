//! Ironflow MCP server.
//!
//! Exposes Ironflow workflow orchestration as MCP tools
//! that Claude (or any MCP client) can call.

// The tool_box! macro generates an enum where every variant ends with `Tool`,
// which is imposed by the #[mcp_tool] macro on each struct.
#![allow(clippy::enum_variant_names)]

mod client;
mod error;
mod handler;
mod tools;

use std::env;
use std::sync::Arc;

use anyhow::Context;
use rust_mcp_sdk::McpServer;
use rust_mcp_sdk::mcp_server::{McpServerOptions, ToMcpServerHandler, server_runtime};
use rust_mcp_sdk::schema::{
    Implementation, InitializeResult, ProtocolVersion, ServerCapabilities, ServerCapabilitiesTools,
};
use rust_mcp_sdk::{StdioTransport, TransportOptions};
use tracing_subscriber::EnvFilter;

use crate::client::ApiClient;
use crate::handler::IronflowHandler;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    let base_url = env::var("IRONFLOW_URL").context("IRONFLOW_URL must be set")?;
    let api_key =
        env::var("IRONFLOW_API_KEY").context("IRONFLOW_API_KEY must be set (irfl_...)")?;

    let client = ApiClient::new(&base_url, api_key);

    let handler = IronflowHandler::new(Arc::new(client));

    let server_details = InitializeResult {
        server_info: Implementation {
            name: "ironflow-mcp".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            title: Some("Ironflow MCP Server".into()),
            description: Some("MCP server for Ironflow workflow orchestration".into()),
            icons: vec![],
            website_url: None,
        },
        capabilities: ServerCapabilities {
            tools: Some(ServerCapabilitiesTools { list_changed: None }),
            ..Default::default()
        },
        protocol_version: ProtocolVersion::V2025_11_25.into(),
        instructions: None,
        meta: None,
    };

    let transport =
        StdioTransport::new(TransportOptions::default()).map_err(|e| anyhow::anyhow!("{e}"))?;

    let server = server_runtime::create_server(McpServerOptions {
        server_details,
        transport,
        handler: handler.to_mcp_server_handler(),
        task_store: None,
        client_task_store: None,
        message_observer: None,
    });

    server.start().await.map_err(|e| anyhow::anyhow!("{e}"))?;

    Ok(())
}
