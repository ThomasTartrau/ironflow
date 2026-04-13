//! MCP tool definitions for Ironflow.
//!
//! Each tool lives in its own file, grouped by domain:
//! - `workflows/` - list and inspect workflows
//! - `runs/` - create, list, and inspect runs
//! - `actions/` - cancel, approve, reject, retry runs
//! - `stats.rs` - aggregated statistics

pub mod actions;
pub mod runs;
pub mod stats;
pub mod workflows;

pub use actions::{ApproveRunTool, CancelRunTool, RejectRunTool, RetryRunTool};
pub use runs::{CreateRunTool, GetRunTool, ListRunsTool};
pub use stats::GetStatsTool;
pub use workflows::{GetWorkflowTool, ListWorkflowsTool};

rust_mcp_sdk::tool_box!(
    IronflowTools,
    [
        ListWorkflowsTool,
        GetWorkflowTool,
        CreateRunTool,
        ListRunsTool,
        GetRunTool,
        CancelRunTool,
        ApproveRunTool,
        RejectRunTool,
        RetryRunTool,
        GetStatsTool
    ]
);
