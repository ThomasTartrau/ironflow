//! Workflow handlers of the developer-experience team.

mod pr_summary;

pub use pr_summary::{PrSummary, PrSummaryInput};

use ironflow_engine::handler::WorkflowHandler;

/// Every workflow handler, registered by both the server and the worker.
pub fn handlers() -> Vec<Box<dyn WorkflowHandler>> {
    vec![Box::new(PrSummary)]
}
