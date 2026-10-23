//! Workflow handlers of the security team.

mod audit_report;

pub use audit_report::{Audit, AuditReport, Finding};

use ironflow_engine::handler::WorkflowHandler;

/// Every workflow handler, registered by both the server and the worker.
pub fn handlers() -> Vec<Box<dyn WorkflowHandler>> {
    vec![Box::new(AuditReport)]
}
