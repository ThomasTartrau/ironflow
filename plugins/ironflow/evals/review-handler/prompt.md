---
description: An English request to review a handler with replay, step-name, injection and secret pitfalls loads the Ironflow hub and reports each pitfall with an Ironflow-specific fix.
tags: [review]
max_turns: 15
allowed_tools: [Read, Glob, Grep, Skill]
---

Review this Ironflow handler before I merge it. List the problems, most severe first, each with the line and the fix. Do not ask me anything and do not write files.

```rust
impl WorkflowHandler for Release {
    fn name(&self) -> &str {
        "release"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let input: ReleaseInput = ctx.input().await?;
            let token = env::var("GITLAB_TOKEN").unwrap_or_default();

            Client::new()
                .post("https://chat.example.com/hooks/releases")
                .json(&json!({"text": format!("release {} starting", input.tag)}))
                .send()
                .await
                .map_err(|e| EngineError::StepConfig(e.to_string()))?;

            let name = format!("build-{}", Utc::now().timestamp());
            ctx.shell(&name, ShellConfig::new("cargo build --release")).await?;

            ctx.approval("ship", ApprovalConfig::new("Publish the release?")).await?;

            ctx.shell(
                "tag",
                ShellConfig::new(&format!("git tag {} && git push origin {}", input.tag, input.tag)),
            )
            .await?;
            ctx.shell(
                "publish",
                ShellConfig::new(&format!("glab release create {} --token {}", input.tag, token)),
            )
            .await?;
            Ok(())
        })
    }
}
```
