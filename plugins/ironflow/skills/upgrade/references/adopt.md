# New features worth adopting

Each entry replaces a workaround. Nothing breaks if the project keeps the workaround, so
propose these one by one, never apply them in bulk. Most of them change the sequence of
steps: see "Steps are replay keys" in the skill before applying.

## wait-for-signal
- kind: adopt
- since: ironflow-engine 2.43.0 (#146)
- detect: `tokio::time::sleep|thread::sleep`
- detect: `ctx\.delay\(`

A handler that polls an external system (CI pipeline, payment, callback) holds a worker
slot and picks an arbitrary timeout. A `sleep` inside a handler is worse: it is not a step,
so it is replayed on resume. Wait for a signal instead; the producer posts it to
`POST /api/v1/signals`, with `ironflow signal send`, or through MCP.

```diff
- for i in 0..40 {
-     let status = ctx.http(&format!("poll-ci-{i}"), HttpConfig::get(&url)).await?;
-     if status.body().contains("success") { break; }
-     ctx.delay(&format!("wait-ci-{i}"), DelayConfig::from_secs(30)).await?;
- }
+ let finished = ctx
+     .wait_for_signal::<PipelineFinished>("wait-ci", &sha, Duration::from_secs(3600))
+     .await?;   // Some(payload), or None on timeout
```

The key identifies one occurrence (a commit SHA, not a merge request). Section: Signal.

## shell-exec
- kind: adopt
- since: ironflow-engine 2.44.1 (#154)
- detect: `ShellConfig::new\(&format!`

`ShellConfig::new` runs its string through `sh -c`, so data interpolated into it (the run
input, a webhook payload, a human answer, an agent output) can close a quote and run any
command on the worker. Pass that data as arguments with `ShellConfig::exec`, which spawns
the program without a shell. A command built only from constants can stay as it is.

```diff
- ctx.shell("greet", ShellConfig::new(&format!("echo 'Hello, {}!'", input.name))).await?;
+ ctx.shell("greet", ShellConfig::exec("printf", &["Hello, %s!\n", &input.name])).await?;
```

Pipes and redirects need a shell: keep `ShellConfig::new` with a fixed script and hand the
data over with `.env("NAME", &input.name)`, read as `"$NAME"`. Section: Shell.

## exit-code-as-output
- kind: adopt
- since: ironflow-engine 2.42.0 (#138)
- detect: `ShellConfig::new\([^)]*([|][|] *true|echo +[$]\?)`

A command whose exit code is data (a conflicting merge, red tests) no longer needs to be
masked in the shell.

```diff
- ctx.shell("merge", ShellConfig::new("git merge feature || true")).await?;
+ let merge = ctx.shell("merge", ShellConfig::new("git merge feature").exit_code_as_output()).await?;
+ if !merge.is_success() { /* merge.exit_code() */ }
```

Section: Shell.

## human-input
- kind: adopt
- since: ironflow-engine 2.40.0 (#109)

An approval gate used to collect data that a person then puts somewhere else (a comment, a
file, a second run) becomes one typed step: `ctx.human_input::<Answers>(name, config)`
returns the struct the person filled in the dashboard form. Section: Human input.

## decision
- kind: adopt
- since: ironflow-engine 2.36.0 (#88)

An agent step whose only job is to classify, route or score (`output::<T>()` where `T` is an
enum or a few numbers) is cheaper and calibrated as a decision step. Needs a decision
provider on the worker (`wiring.md#decision-provider`). Section: Decision.

## append-system-prompt
- kind: adopt
- since: ironflow-core 4.4.0
- detect: `\.system_prompt\(`

On the Claude Code CLI provider, `system_prompt(..)` replaces Claude Code's own prompt
(skills, slash commands). To add project rules on top of it, use
`append_system_prompt(..)`. Keep `system_prompt` when replacing it is the intent. Section:
Agent.

## workflow-allow-failure
- kind: adopt
- since: ironflow-engine 2.46.0 (#161)

A parent that must go on when a child workflow fails used to catch the child error
around `ctx.workflow(..)`. That is a replay hazard: the failed step is not completed, so a
resume of the parent runs the child again and creates a second child run. Start the child
with `ctx.workflow_with(.., WorkflowOptions::new().allow_failure())` instead: the child run
is still marked failed, the step completes, `status()` is `Failed` (or `Cancelled`) and
`error()` carries the message, and the parent ends as `Warning`. A suspension of the child
is never tolerated. Section: Sub-workflow.

```diff
- let child = match ctx.workflow(&Collect, input).await {
-     Ok(child) => Some(child),
-     Err(_) => None,
- };
+ let child = ctx
+     .workflow_with(&Collect, input, WorkflowOptions::new().allow_failure())
+     .await?;
+ if child.status() == RunStatus::Failed {
+     // child.error() holds the reason
+ }
```
