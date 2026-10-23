use ironflow_engine::config::ShellConfig;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};

/// Smoke-test workflow: prints a greeting on the worker.
pub struct Hello;

impl WorkflowHandler for Hello {
    fn name(&self) -> &str {
        "hello"
    }

    fn description(&self) -> &str {
        "Print a greeting."
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.shell("greet", ShellConfig::new("echo 'Hello from Ironflow'"))
                .await?;
            Ok(())
        })
    }
}
