//! Example workflow handlers for ironflow.
//!
//! This crate provides reusable workflow handlers that can be registered
//! in both the API server (for metadata/describe) and the worker (for execution).

mod agent_showcase;
mod ci_pipeline;
mod deploy_approval;
mod git_insight;
mod notified_pipeline;
mod pipeline;
mod system_audit;
mod weather_report;

pub use agent_showcase::AgentShowcase;
pub use ci_pipeline::CiPipeline;
pub use deploy_approval::DeployApproval;
pub use git_insight::GitInsight;
pub use notified_pipeline::NotifiedPipeline;
pub use pipeline::{Collect, Enrich, Report};
pub use system_audit::SystemAudit;
pub use weather_report::WeatherReport;

use ironflow_engine::engine::Engine;
use ironflow_engine::error::EngineError;

/// Register all example workflow handlers in the engine.
///
/// # Errors
///
/// Returns [`EngineError::InvalidWorkflow`] if a handler with the same name
/// is already registered.
pub fn register_all(engine: &mut Engine) -> Result<(), EngineError> {
    engine.register(WeatherReport)?;
    engine.register(SystemAudit)?;
    engine.register(GitInsight)?;
    engine.register(Collect)?;
    engine.register(Enrich)?;
    engine.register(Report)?;
    engine.register(CiPipeline)?;
    engine.register(DeployApproval)?;
    engine.register(NotifiedPipeline)?;
    engine.register(AgentShowcase)?;
    Ok(())
}
