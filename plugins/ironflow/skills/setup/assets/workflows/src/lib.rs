//! Workflow handlers for this project.
//!
//! [`handlers`] is the single list both the API server and the workers
//! register, so the two binaries cannot disagree on which workflows exist.
//! Add every new handler there.

mod hello;

pub use hello::{Hello, HelloInput};

use ironflow_engine::handler::WorkflowHandler;

/// Every workflow handler of this project, boxed.
pub fn handlers() -> Vec<Box<dyn WorkflowHandler>> {
    vec![Box::new(Hello)]
}
