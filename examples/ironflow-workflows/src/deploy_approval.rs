//! Deploy workflow with human approval gate before production.

use ironflow_engine::config::{ApprovalConfig, ShellConfig};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler, WorkflowInfo};

/// Deploy pipeline that requires human approval before shipping to production.
///
/// 1. **build** -- compile the project
/// 2. **test** -- run the test suite
/// 3. **deploy-staging** -- deploy to staging environment
/// 4. **approval gate** -- pause and wait for human approval
/// 5. **deploy-production** -- resumes after approval via step replay
///
/// After approval (POST /api/v1/runs/:id/approve), the engine
/// re-executes the handler: completed steps return cached output,
/// the approved gate is skipped, and execution continues with
/// deploy-production.
///
/// If the approval is rejected, the run transitions to `Failed`.
/// If cancelled, the run transitions to `Cancelled`.
pub struct DeployApproval;

impl WorkflowHandler for DeployApproval {
    fn name(&self) -> &str {
        "deploy-approval"
    }

    fn describe(&self) -> WorkflowInfo {
        WorkflowInfo {
            description: "Deploy pipeline with human approval gate before production. \
                          Demonstrates ctx.approval() for human-in-the-loop workflows."
                .to_string(),
            source_code: Some(include_str!("deploy_approval.rs").to_string()),
            sub_workflows: Vec::new(),
            category: None,
        }
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            // Step 1: Build
            ctx.shell(
                "build",
                ShellConfig::new("echo 'Compiling...' && sleep 0.2 && echo 'Build OK'"),
            )
            .await?;

            // Step 2: Test
            ctx.shell(
                "test",
                ShellConfig::new("echo 'Running tests...' && sleep 0.3 && echo '87 tests passed'"),
            )
            .await?;

            // Step 3: Deploy to staging
            ctx.shell(
                "deploy-staging",
                ShellConfig::new(
                    "echo 'Deploying to staging...' && sleep 0.2 && echo 'Staging live'",
                ),
            )
            .await?;

            // Step 4: Human approval gate
            // The run pauses here and transitions to AwaitingApproval.
            // A human must call POST /api/v1/runs/:id/approve to continue,
            // or POST /api/v1/runs/:id/reject to fail the run.
            ctx.approval(
                "prod-approval",
                ApprovalConfig::new("Staging looks good. Deploy to production?")
                    .with_timeout_seconds(3600),
            )
            .await?;

            // Step 5: Deploy to production (only reached after approval)
            ctx.shell(
                "deploy-production",
                ShellConfig::new(
                    "echo 'Deploying to production...' && sleep 0.3 && echo 'Production live'",
                ),
            )
            .await?;

            Ok(())
        })
    }
}
