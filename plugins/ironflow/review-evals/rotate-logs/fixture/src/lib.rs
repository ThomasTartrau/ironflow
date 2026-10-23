//! Workflow handlers of the operations team.

mod purge_cache;
mod rotate_logs;

pub use purge_cache::PurgeCache;
pub use rotate_logs::RotateLogs;

use ironflow_engine::handler::WorkflowHandler;

/// Every workflow handler, registered by both the server and the worker.
pub fn handlers() -> Vec<Box<dyn WorkflowHandler>> {
    vec![Box::new(PurgeCache), Box::new(RotateLogs)]
}
