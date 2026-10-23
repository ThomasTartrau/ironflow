//! Workflow handlers of the on-call team.

mod hotfix;

pub use hotfix::{Hotfix, HotfixInput};

use ironflow_engine::handler::WorkflowHandler;

/// Every workflow handler, registered by both the server and the worker.
pub fn handlers() -> Vec<Box<dyn WorkflowHandler>> {
    vec![Box::new(Hotfix)]
}
