---
description: A French request to migrate pre-2.39 handler code loads the upgrade skill and gets the typed approvers, typed condition, Tool enum, typed accessors and agent output, with step names unchanged.
tags: [upgrade]
max_turns: 15
allowed_tools: [Read, Glob, Grep, Skill]
---

J'ai bumpé ironflow-engine d'une 2.3x à la dernière version et ce morceau de handler ne compile plus. Migre-le vers l'API actuelle, sans changer son comportement ni les noms de ses étapes.

```rust
let payload = ctx.payload().await?;
let build = ctx.shell("build", ShellConfig::new("cargo build")).await?;
let log = build.output.get("stdout").and_then(|v| v.as_str()).unwrap_or("");
if ctx.when("input.env == 'prod'", |p| p["env"] == "prod").await? {
    ctx.approval(
        "pay",
        ApprovalConfig::new("Release the payment?")
            .with_rule(ApprovalRule::new("payload.amount > 10000", 2).with_approver_groups(["finance"])),
    )
    .await?;
}
let review = ctx
    .agent("review", AgentStepConfig::new(log).allow_tool("Read").output::<Review>())
    .await?;
let verdict: Review = review.json()?;
```

Ne me pose pas de question et n'écris aucun fichier : donne le code migré dans ta réponse.
