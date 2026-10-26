//! Workflow handlers of the CI team.

mod repair_pipeline;

pub use repair_pipeline::{RepairPipeline, RepairPipelineInput};

use ironflow_engine::handler::WorkflowHandler;

/// Every workflow handler, registered by both the server and the worker.
pub fn handlers() -> Vec<Box<dyn WorkflowHandler>> {
    vec![Box::new(RepairPipeline)]
}
