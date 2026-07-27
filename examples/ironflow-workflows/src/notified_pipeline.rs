//! Pipeline workflow with webhook notifications on completion and failure.

use ironflow_engine::config::ShellConfig;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler, WorkflowInfo};
use std::collections::HashMap;

/// Build-and-deploy pipeline that demonstrates outbound notifications.
///
/// The workflow itself is a simple build/test/deploy sequence.
/// Notifications are configured at the engine level -- see the server
/// example for how to wire up a [`WebhookSubscriber`](ironflow_engine::notify::WebhookSubscriber):
///
/// ```rust,no_run
/// use ironflow_engine::engine::Engine;
/// use ironflow_engine::notify::{Event, WebhookSubscriber};
///
/// # fn example(engine: &mut Engine) {
/// engine.subscribe(
///     WebhookSubscriber::new("https://hooks.example.com/ironflow"),
///     &[Event::RUN_STATUS_CHANGED, Event::STEP_FAILED],
/// );
/// # }
/// ```
///
/// When this workflow completes or fails, the engine publishes an
/// [`Event::RunStatusChanged`](ironflow_engine::notify::Event::RunStatusChanged) and the
/// `WebhookSubscriber` POSTs the JSON payload to the configured URL.
pub struct NotifiedPipeline;

impl WorkflowHandler for NotifiedPipeline {
    fn name(&self) -> &str {
        "notified-pipeline"
    }

    fn describe(&self) -> WorkflowInfo {
        WorkflowInfo {
            description: "Build/test/deploy pipeline with outbound webhook notifications. \
                          Demonstrates Engine::subscribe() with WebhookSubscriber."
                .to_string(),
            source_code: Some(include_str!("notified_pipeline.rs").to_string()),
            sub_workflows: Vec::new(),
            category: None,
            version: self.version().map(str::to_string),
            input_schema: None,
            default_labels: HashMap::new(),
            schedule: None,
            default_max_cost_usd: None,
        }
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.shell(
                "build",
                ShellConfig::new("echo 'Building...' && sleep 0.2 && echo 'Build OK'"),
            )
            .await?;

            ctx.shell(
                "test",
                ShellConfig::new("echo 'Running tests...' && sleep 0.3 && echo '64 tests passed'"),
            )
            .await?;

            ctx.shell(
                "deploy",
                ShellConfig::new(
                    "echo 'Deploying...' && sleep 0.2 && echo 'Deployed to production'",
                ),
            )
            .await?;

            Ok(())
        })
    }
}
