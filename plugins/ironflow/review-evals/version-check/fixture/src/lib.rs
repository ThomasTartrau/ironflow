//! Workflow handlers of the release team.

mod version_check;

pub use version_check::VersionCheck;

use ironflow_engine::handler::WorkflowHandler;

/// Every workflow handler, registered by both the server and the worker.
pub fn handlers() -> Vec<Box<dyn WorkflowHandler>> {
    vec![Box::new(VersionCheck)]
}
