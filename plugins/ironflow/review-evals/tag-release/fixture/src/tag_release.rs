use ironflow_engine::config::{ApprovalConfig, ShellConfig};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::error::EngineError;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler, input_schema_for};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

/// Semver component to bump.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum BumpLevel {
    /// Bug fixes only.
    Patch,
    /// New features, backward compatible.
    Minor,
    /// Breaking changes.
    Major,
}

/// Input of the `tag-release` workflow.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct TagReleaseInput {
    /// Component to bump.
    pub level: BumpLevel,
    /// Release-candidate number, 0 for a final release.
    #[serde(default)]
    pub rc: u32,
}

/// Bumps the version, waits for approval, pushes the tag.
pub struct TagRelease;

impl WorkflowHandler for TagRelease {
    fn name(&self) -> &str {
        "tag-release"
    }

    fn description(&self) -> &str {
        "Bump the version and push the release tag after approval."
    }

    fn input_schema(&self) -> Option<Value> {
        Some(input_schema_for::<TagReleaseInput>())
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let input: TagReleaseInput = ctx.input().await?;

            let level = match input.level {
                BumpLevel::Patch => "patch",
                BumpLevel::Minor => "minor",
                BumpLevel::Major => "major",
            };

            let bump = ctx
                .shell(
                    "bump",
                    ShellConfig::new(&format!("./scripts/bump.sh {level} --rc {}", input.rc)),
                )
                .await?;

            ctx.approval(
                "approve-tag",
                ApprovalConfig::new(&format!("Push tag {}?", bump.stdout().trim())),
            )
            .await?;

            let token = ctx
                .secrets()?
                .get("gitlab_token")
                .await
                .map_err(EngineError::Store)?
                .ok_or_else(|| {
                    EngineError::StepConfig("secret gitlab_token missing".to_string())
                })?;

            ctx.shell(
                "push-tag",
                ShellConfig::new("git push origin --tags").env("GITLAB_TOKEN", &token.value),
            )
            .await?;

            Ok(())
        })
    }
}
