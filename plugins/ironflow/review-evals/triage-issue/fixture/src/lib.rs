//! Workflow handlers of the maintainers.

mod triage_issue;

pub use triage_issue::{LabelChoice, Triage, TriageIssue, TriageIssueInput};

use ironflow_engine::handler::WorkflowHandler;

/// Every workflow handler, registered by both the server and the worker.
pub fn handlers() -> Vec<Box<dyn WorkflowHandler>> {
    vec![Box::new(TriageIssue)]
}
