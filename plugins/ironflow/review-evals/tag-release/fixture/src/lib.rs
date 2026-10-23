//! Workflow handlers of the release team.

mod tag_release;

pub use tag_release::{BumpLevel, TagRelease, TagReleaseInput};

use ironflow_engine::handler::WorkflowHandler;

/// Every workflow handler, registered by both the server and the worker.
pub fn handlers() -> Vec<Box<dyn WorkflowHandler>> {
    vec![Box::new(TagRelease)]
}
