//! Read a persisted step back as a [`StepOutput`].

use serde_json::Value;

use ironflow_store::entities::Step;

use super::{StepArtifacts, StepOutput};

/// View a persisted step through the typed [`StepOutput`] accessors.
///
/// Useful to read the steps of a sub-workflow run, listed from the store by
/// [`SubWorkflowOutput::run_id`](super::SubWorkflowOutput::run_id). A step
/// without output (still running, failed, skipped) reads as an empty output.
/// The model and the debug conversation are not carried over.
///
/// # Examples
///
/// ```no_run
/// use ironflow_engine::context::WorkflowContext;
/// use ironflow_engine::error::EngineError;
/// use ironflow_engine::executor::{StepOutput, SubWorkflowOutput};
///
/// # async fn example(ctx: &WorkflowContext, child: &SubWorkflowOutput) -> Result<(), EngineError> {
/// for step in ctx.store().list_steps(child.run_id()).await? {
///     println!("{}: {}", step.name, StepOutput::from(&step).stdout());
/// }
/// # Ok(())
/// # }
/// ```
impl From<&Step> for StepOutput {
    fn from(step: &Step) -> Self {
        Self {
            output: step.output.clone().unwrap_or(Value::Null),
            duration_ms: step.duration_ms,
            cost_usd: step.cost_usd,
            input_tokens: step.input_tokens,
            output_tokens: step.output_tokens,
            model: None,
            debug_messages: None,
            artifacts: StepArtifacts::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use ironflow_store::entities::{
        NewRun, NewStep, StepKind, StepStatus, StepUpdate, TriggerKind,
    };
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::store::RunStore;
    use rust_decimal::Decimal;
    use serde_json::json;
    use uuid::Uuid;

    use super::*;

    /// A shell step of a fresh run, completed with `output` when given.
    async fn stored_step(output: Option<Value>) -> Step {
        let store = InMemoryStore::new();
        let run = store
            .create_run(NewRun {
                created_by: None,
                workflow_name: "collect".to_string(),
                trigger: TriggerKind::Manual,
                payload: json!({}),
                max_retries: 0,
                handler_version: None,
                labels: HashMap::new(),
                scheduled_at: None,
                idempotency_key: None,
                max_cost_usd: None,
            })
            .await
            .expect("create run")
            .into_run();
        let step = store
            .create_step(NewStep {
                run_id: run.id,
                trace_id: Uuid::now_v7(),
                name: "disk".to_string(),
                kind: StepKind::Shell,
                position: 0,
                input: None,
                is_error_handler: false,
            })
            .await
            .expect("create step");
        store
            .update_step(
                step.id,
                StepUpdate {
                    status: Some(StepStatus::Running),
                    ..StepUpdate::default()
                },
            )
            .await
            .expect("to running");
        if output.is_some() {
            store
                .update_step(
                    step.id,
                    StepUpdate {
                        status: Some(StepStatus::Completed),
                        output,
                        duration_ms: Some(12),
                        cost_usd: Some(Decimal::new(3, 2)),
                        ..StepUpdate::default()
                    },
                )
                .await
                .expect("to completed");
        }
        store.get_step(step.id).await.expect("get").expect("exists")
    }

    #[tokio::test]
    async fn a_stored_shell_step_reads_through_the_accessors() {
        let step = stored_step(Some(
            json!({"stdout": "42%\n", "stderr": "", "exit_code": 0}),
        ))
        .await;

        let output = StepOutput::from(&step);

        assert_eq!(output.stdout(), "42%\n");
        assert_eq!(output.exit_code(), Some(0));
        assert!(output.is_success());
        assert_eq!(output.duration_ms, 12);
        assert_eq!(output.cost_usd, Decimal::new(3, 2));
    }

    #[tokio::test]
    async fn a_step_without_output_reads_as_empty() {
        let step = stored_step(None).await;

        let output = StepOutput::from(&step);

        assert_eq!(output.output, Value::Null);
        assert_eq!(output.stdout(), "");
        assert_eq!(output.exit_code(), None);
        assert!(!output.is_success());
    }
}
