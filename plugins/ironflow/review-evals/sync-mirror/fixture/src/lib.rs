//! Workflow handlers of the open-source team.

mod sync_mirror;

pub use sync_mirror::SyncMirror;

use ironflow_engine::handler::WorkflowHandler;

/// Every workflow handler, registered by both the server and the worker.
pub fn handlers() -> Vec<Box<dyn WorkflowHandler>> {
    vec![Box::new(SyncMirror)]
}
