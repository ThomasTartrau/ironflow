//! Error handlers registered on [`WorkflowContext`].
//!
//! A handler registered with [`on_error`](WorkflowContext::on_error) fires once
//! when a later step fails. Execution is best-effort: a handler failure is
//! logged and never replaces the error that triggered it.

use std::mem::take;
use std::time::Instant;

use chrono::Utc;
use serde_json::json;
use tracing::{info, warn};

use ironflow_store::models::{NewStep, StepStatus, StepUpdate, step_trace_id};

use crate::config::StepConfig;
use crate::executor::execute_step_config;
use crate::log_sender::StepLogSender;

use super::{OnErrorHandler, WorkflowContext};

impl WorkflowContext {
    /// Register an error handler that fires when any subsequent step fails.
    ///
    /// The handler is consumed after firing (fire-once). Multiple handlers
    /// can be registered; they fire in registration order.
    ///
    /// Error handler execution is best-effort: if a handler fails, the error
    /// is logged but the original step error is preserved. Error handler steps
    /// appear in the run timeline with
    /// [`Step::is_error_handler`](ironflow_store::models::Step::is_error_handler)
    /// set to `true`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::context::WorkflowContext;
    /// use ironflow_engine::config::ShellConfig;
    /// use ironflow_engine::error::EngineError;
    ///
    /// # async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    /// ctx.on_error("cleanup", ShellConfig::new("rm -rf /tmp/build"));
    /// ctx.shell("build", ShellConfig::new("cargo build")).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn on_error(&mut self, name: &str, config: impl Into<StepConfig>) {
        self.error_handlers.push(OnErrorHandler {
            name: name.to_string(),
            config: config.into(),
        });
    }

    /// Remove all registered error handlers.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_engine::context::WorkflowContext;
    /// use ironflow_engine::config::ShellConfig;
    /// use ironflow_engine::error::EngineError;
    ///
    /// # async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    /// ctx.on_error("cleanup", ShellConfig::new("rm -rf /tmp/build"));
    /// ctx.shell("build", ShellConfig::new("cargo build")).await?;
    /// ctx.clear_error_handlers();
    /// // cleanup will NOT fire if deploy fails
    /// ctx.shell("deploy", ShellConfig::new("./deploy.sh")).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn clear_error_handlers(&mut self) {
        self.error_handlers.clear();
    }

    /// Execute all registered error handlers after a step failure.
    ///
    /// Drains the handler list (fire-once). Each handler creates its own
    /// step record with `is_error_handler = true`. Handler failures are
    /// logged but never propagated.
    pub(super) async fn fire_error_handlers(
        &mut self,
        failed_step_name: &str,
        error_msg: &str,
        duration_ms: u64,
    ) {
        let handlers = take(&mut self.error_handlers);
        if handlers.is_empty() {
            return;
        }

        let error_context = json!({
            "failed_step": failed_step_name,
            "error": error_msg,
            "duration_ms": duration_ms,
        });

        for handler in handlers {
            let mut config = handler.config.clone();
            inject_error_context(&mut config, failed_step_name, error_msg, duration_ms);

            let position = self.position;
            self.position += 1;

            let trace_id = step_trace_id(self.run_id, &handler.name, position);
            let step = match self
                .store
                .create_step(NewStep {
                    run_id: self.run_id,
                    trace_id,
                    name: handler.name.clone(),
                    kind: config.kind(),
                    position,
                    input: Some(error_context.clone()),
                    is_error_handler: true,
                })
                .await
            {
                Ok(step) => step,
                Err(err) => {
                    warn!(
                        run_id = %self.run_id,
                        handler = %handler.name,
                        error = %err,
                        "failed to create error handler step"
                    );
                    continue;
                }
            };

            if let Err(err) = self.start_step(step.id, Utc::now()).await {
                warn!(
                    run_id = %self.run_id,
                    handler = %handler.name,
                    error = %err,
                    "failed to start error handler step"
                );
                continue;
            }

            let step_log_sender = self
                .log_sender
                .as_ref()
                .map(|s| StepLogSender::new(s.clone(), self.run_id, step.id, handler.name.clone()));

            let start = Instant::now();
            let result = execute_step_config(&config, &self.provider, step_log_sender).await;
            let handler_duration = start.elapsed().as_millis() as u64;
            let completed_at = Utc::now();

            match result {
                Ok(output) => {
                    if let Err(store_err) = self
                        .store
                        .update_step(
                            step.id,
                            StepUpdate {
                                status: Some(StepStatus::Completed),
                                output: Some(output.output),
                                duration_ms: Some(handler_duration),
                                cost_usd: Some(output.cost_usd),
                                completed_at: Some(completed_at),
                                ..StepUpdate::default()
                            },
                        )
                        .await
                    {
                        warn!(
                            run_id = %self.run_id,
                            handler = %handler.name,
                            error = %store_err,
                            "failed to persist error handler completion"
                        );
                    }

                    info!(
                        run_id = %self.run_id,
                        handler = %handler.name,
                        duration_ms = handler_duration,
                        "error handler completed"
                    );
                }
                Err(err) => {
                    if let Err(store_err) = self
                        .store
                        .update_step(
                            step.id,
                            StepUpdate {
                                status: Some(StepStatus::Failed),
                                error: Some(err.to_string()),
                                duration_ms: Some(handler_duration),
                                completed_at: Some(completed_at),
                                ..StepUpdate::default()
                            },
                        )
                        .await
                    {
                        warn!(
                            run_id = %self.run_id,
                            handler = %handler.name,
                            error = %store_err,
                            "failed to persist error handler failure"
                        );
                    }

                    warn!(
                        run_id = %self.run_id,
                        handler = %handler.name,
                        error = %err,
                        "error handler failed (original error preserved)"
                    );
                }
            }
        }
    }
}

/// Inject error context into a step config before executing it as an error handler.
fn inject_error_context(
    config: &mut StepConfig,
    failed_step: &str,
    error_msg: &str,
    duration_ms: u64,
) {
    match config {
        StepConfig::Shell(shell) => {
            shell
                .env
                .push(("IRONFLOW_ERROR_STEP".to_string(), failed_step.to_string()));
            shell
                .env
                .push(("IRONFLOW_ERROR_MESSAGE".to_string(), error_msg.to_string()));
            shell.env.push((
                "IRONFLOW_ERROR_DURATION_MS".to_string(),
                duration_ms.to_string(),
            ));
        }
        StepConfig::Agent(agent) => {
            agent.prompt = format!(
                "[Error Context]\nStep \"{}\" failed after {}ms:\n{}\n\n{}",
                failed_step, duration_ms, error_msg, agent.prompt
            );
        }
        StepConfig::Http(http) => {
            http.headers
                .push(("X-Ironflow-Error-Step".to_string(), failed_step.to_string()));
            http.headers.push((
                "X-Ironflow-Error-Message".to_string(),
                error_msg.to_string(),
            ));
        }
        StepConfig::Workflow(_)
        | StepConfig::Approval(_)
        | StepConfig::Decision(_)
        | StepConfig::Delay(_) => {}
    }
}
