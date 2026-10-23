//! Workflow handlers of the release team.

mod release_gate;

pub use release_gate::ReleaseGate;

use ironflow_engine::handler::WorkflowHandler;

/// Every workflow handler, registered by both the server and the worker.
pub fn handlers() -> Vec<Box<dyn WorkflowHandler>> {
    vec![Box::new(ReleaseGate)]
}
