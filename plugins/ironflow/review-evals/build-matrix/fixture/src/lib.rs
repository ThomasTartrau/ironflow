//! Workflow handlers of the release team.

mod build_matrix;

pub use build_matrix::BuildMatrix;

use ironflow_engine::handler::WorkflowHandler;

/// Every workflow handler, registered by both the server and the worker.
pub fn handlers() -> Vec<Box<dyn WorkflowHandler>> {
    vec![Box::new(BuildMatrix)]
}
