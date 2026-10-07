# Artifacts

Steps produce text and JSON outputs by default. When a step produces *files*, declare them:
they are stored with their size, MIME type and SHA-256, then made available to later steps
and to the dashboard.

```rust,no_run
use ironflow_engine::config::ShellConfig;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::error::EngineError;

async fn release(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    // Produce: every glob match becomes an artifact named after the file.
    let build = ctx
        .shell(
            "build",
            ShellConfig::new("cargo build --release && ./gen-report")
                .dir("/app")
                .output("target/report.html")
                .output("target/*.log"),
        )
        .await?;

    // The producing step hands out the handle; a name it did not declare fails here.
    let report = build.artifact("report.html")?;

    // Consume: the file is written into the working directory before the command runs.
    ctx.shell(
        "publish",
        ShellConfig::new("./publish report.html").dir("/app").input(&report),
    )
    .await?;
    Ok(())
}
```

`ctx.put_artifact(&step_output, ..)` and `ctx.get_artifact(&handle)` cover custom operations
and agent steps, which have no working directory to collect files from.

| Behaviour | Rule |
|---|---|
| Declared output matched no file | The step fails, unless the step had already failed for another reason |
| Step failed but produced files | The files are still collected: they are usually what you need to debug |
| Input resolution | Same run and attempt, steps positioned before the consumer, closest producer wins |
| Retry | Each attempt owns its artifacts; nothing is overwritten |
| Sub-workflow | A child run never sees its parent's artifacts: pass what it needs through the payload |
| Name validation | `^[A-Za-z0-9._][A-Za-z0-9._-]{0,254}$`; the storage key is derived from UUIDs only |

Download one with `GET /api/v1/runs/{id}/steps/{step_id}/artifacts/{name}`, or from the
artifact list on the step in the dashboard.

## Enabling artifacts

Artifacts stay off until `ARTIFACTS_DIR` is set on the API server. Until then, the artifact
routes answer `501` and a step that declares one fails with an explicit error; everything else
works. `ARTIFACT_MAX_BYTES` caps the size of one file (100 MiB by default).

`LocalBlobStore` writes to the API server's filesystem, so a deployment with several API
replicas needs a shared volume. Workers hold no storage credential: they stream artifact bytes
through the internal API.
