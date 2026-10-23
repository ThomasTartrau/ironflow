//! Workflow handlers of the platform team.

mod backup_db;
mod cleanup_branches;
mod hello;

pub use backup_db::BackupDb;
pub use cleanup_branches::CleanupBranches;
pub use hello::Hello;

use ironflow_engine::handler::WorkflowHandler;

/// Every workflow handler, registered by both the server and the worker.
pub fn handlers() -> Vec<Box<dyn WorkflowHandler>> {
    vec![Box::new(Hello), Box::new(BackupDb)]
}
