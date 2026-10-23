//! Workflow handlers of the platform team.

mod notify_deploy;

pub use notify_deploy::{NotifyDeploy, NotifyDeployInput};

use ironflow_engine::handler::WorkflowHandler;

/// Every workflow handler, registered by both the server and the worker.
pub fn handlers() -> Vec<Box<dyn WorkflowHandler>> {
    vec![Box::new(NotifyDeploy)]
}
