//! Workflow handlers of the open-source team.

mod publish_crate;

pub use publish_crate::PublishCrate;

use ironflow_engine::handler::WorkflowHandler;

/// Every workflow handler, registered by both the server and the worker.
pub fn handlers() -> Vec<Box<dyn WorkflowHandler>> {
    vec![Box::new(PublishCrate)]
}
