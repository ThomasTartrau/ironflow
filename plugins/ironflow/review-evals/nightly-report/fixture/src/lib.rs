//! Workflow handlers of the data team.

mod nightly_report;

pub use nightly_report::NightlyReport;

use ironflow_engine::handler::WorkflowHandler;

/// Every workflow handler, registered by both the server and the worker.
pub fn handlers() -> Vec<Box<dyn WorkflowHandler>> {
    vec![Box::new(NightlyReport)]
}
