//! The engine stamps `ironflow.io/run-id` and `ironflow.io/step` on the
//! config of every agent step, sequential or parallel, so the K8s ephemeral
//! provider can tag its pods and find a previous attempt of the same step.
//!
//! A real provider records every config it receives; the tests assert on
//! what it saw.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::json;
use tokio::time::timeout;

use ironflow_core::provider::{
    AgentConfig, AgentOutput, AgentProvider, InvokeFuture, LABEL_RUN_ID, LABEL_STEP,
    sanitize_label_value,
};
use ironflow_engine::config::{AgentStepConfig, StepConfig};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_engine::testing::TestEngine;
use ironflow_store::models::RunStatus;

const TEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Provider answering "ok" and keeping every config it was invoked with.
#[derive(Default)]
struct RecordingProvider {
    seen: Mutex<Vec<AgentConfig>>,
}

impl AgentProvider for RecordingProvider {
    fn invoke<'a>(&'a self, config: &'a AgentConfig) -> InvokeFuture<'a> {
        Box::pin(async move {
            self.seen.lock().expect("lock").push(config.clone());
            Ok(AgentOutput::new(json!("ok")))
        })
    }
}

impl RecordingProvider {
    /// The labels of the config whose prompt is `prompt`.
    fn labels_for(&self, prompt: &str) -> BTreeMap<String, String> {
        let seen = self.seen.lock().expect("lock");
        let config = seen.iter().find(|c| c.prompt == prompt);
        config.expect("provider saw the prompt").pod_labels.clone()
    }
}

fn step(prompt: &str) -> AgentStepConfig {
    AgentStepConfig::new(prompt).max_budget_usd(0.10)
}

struct Labelled;

impl WorkflowHandler for Labelled {
    fn name(&self) -> &str {
        "labelled"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.agent("investigate", step("sequential").pod_label("team", "infra"))
                .await?;
            ctx.parallel(
                vec![
                    ("review api/v2", StepConfig::Agent(step("parallel-a"))),
                    ("lint", StepConfig::Agent(step("parallel-b"))),
                ],
                true,
            )
            .await?;
            Ok(())
        })
    }
}

fn label<'l>(labels: &'l BTreeMap<String, String>, key: &str) -> Option<&'l str> {
    labels.get(key).map(String::as_str)
}

#[tokio::test]
async fn k8s_pod_labels_stamped_on_sequential_and_parallel_agent_steps() {
    timeout(TEST_TIMEOUT, async {
        let provider = Arc::new(RecordingProvider::default());
        let result = TestEngine::new()
            .with_handler(Labelled)
            .with_agent_provider(provider.clone())
            .run(json!({}))
            .await
            .expect("the harness ran the handler");

        assert_eq!(result.status(), RunStatus::Completed);
        let run_id = result.run_id().to_string();

        let sequential = provider.labels_for("sequential");
        assert_eq!(label(&sequential, LABEL_RUN_ID), Some(run_id.as_str()));
        assert_eq!(label(&sequential, LABEL_STEP), Some("investigate"));
        assert_eq!(label(&sequential, "team"), Some("infra"));

        let parallel_a = provider.labels_for("parallel-a");
        let expected_step = sanitize_label_value("review api/v2");
        assert_eq!(label(&parallel_a, LABEL_RUN_ID), Some(run_id.as_str()));
        assert_eq!(label(&parallel_a, LABEL_STEP), Some(expected_step.as_str()));
        assert_ne!(expected_step, "review api/v2");

        let parallel_b = provider.labels_for("parallel-b");
        assert_eq!(label(&parallel_b, LABEL_RUN_ID), Some(run_id.as_str()));
        assert_eq!(label(&parallel_b, LABEL_STEP), Some("lint"));
    })
    .await
    .expect("test timed out");
}
