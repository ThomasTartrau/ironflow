//! Example workflow handlers for ironflow.
//!
//! This crate provides reusable workflow handlers that can be registered
//! in both the API server (for metadata/describe) and the worker (for execution).

mod agent_showcase;
mod ci_pipeline;
mod deploy_approval;
mod git_insight;
mod greeting;
mod notified_pipeline;
mod pipeline;
mod secret_demo;
mod system_audit;
mod weather_report;

pub use agent_showcase::AgentShowcase;
pub use ci_pipeline::CiPipeline;
pub use deploy_approval::DeployApproval;
pub use git_insight::GitInsight;
pub use greeting::Greeting;
pub use notified_pipeline::NotifiedPipeline;
pub use pipeline::{Collect, Enrich, Report};
pub use secret_demo::SecretDemo;
pub use system_audit::SystemAudit;
pub use weather_report::WeatherReport;

use ironflow_engine::engine::Engine;
use ironflow_engine::error::EngineError;
use ironflow_engine::handler::WorkflowHandler;

/// Every example workflow handler, boxed.
///
/// This is the single list both the API server and the worker consume, so
/// the two binaries cannot disagree on which workflows exist.
pub fn handlers() -> Vec<Box<dyn WorkflowHandler>> {
    vec![
        Box::new(WeatherReport),
        Box::new(SystemAudit),
        Box::new(GitInsight),
        Box::new(Collect),
        Box::new(Enrich),
        Box::new(Report),
        Box::new(CiPipeline),
        Box::new(DeployApproval),
        Box::new(NotifiedPipeline),
        Box::new(AgentShowcase),
        Box::new(SecretDemo),
        Box::new(Greeting),
    ]
}

/// Register all example workflow handlers in the engine.
///
/// # Errors
///
/// Returns [`EngineError::InvalidWorkflow`] if a handler with the same name
/// is already registered.
pub fn register_all(engine: &mut Engine) -> Result<(), EngineError> {
    for handler in handlers() {
        engine.register(handler)?;
    }
    Ok(())
}
