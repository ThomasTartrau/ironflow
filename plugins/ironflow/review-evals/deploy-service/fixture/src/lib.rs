//! Workflow handlers of the platform team.

mod deploy_service;

pub use deploy_service::{DeployService, DeployServiceInput};

use ironflow_engine::handler::WorkflowHandler;

/// Every workflow handler, registered by both the server and the worker.
pub fn handlers() -> Vec<Box<dyn WorkflowHandler>> {
    vec![Box::new(DeployService)]
}
