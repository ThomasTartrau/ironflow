//! Workflow handlers of the CI team.

mod checkout_branch;

pub use checkout_branch::{CheckoutBranch, CheckoutBranchInput};

use ironflow_engine::handler::WorkflowHandler;

/// Every workflow handler, registered by both the server and the worker.
pub fn handlers() -> Vec<Box<dyn WorkflowHandler>> {
    vec![Box::new(CheckoutBranch)]
}
