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

## concurrency-key
- kind: adopt
- since: ironflow-engine after 2.45.0 (#162)

A handler or a client that lists active runs before starting one, to keep a single run per
issue or branch, races: two callers can both see nothing and both start. Pass a
concurrency key instead; the store refuses the second creation atomically. For a child,
`ctx.workflow_with` completes the step with the run that holds the key; on
`POST /api/v1/runs` (`concurrency_key` in the body, `--concurrency-key` in the CLI) the
call answers `409 CONCURRENCY_CONFLICT`. Section: Sub-workflow.

```diff
- let active = ctx.store().list_runs(filter, 0, 1).await?;
- if active.items.is_empty() {
-     ctx.workflow(&FixIssue, FixInput { issue }).await?;
- }
+ let options = WorkflowOptions::new().concurrency_key(format!("issue:{issue}"));
+ match ctx.workflow_with(&FixIssue, FixInput { issue }, options).await? {
+     SubWorkflowOutcome::Completed(child) => { /* child.run_id() */ }
+     SubWorkflowOutcome::Conflict(conflict) => { /* conflict.run_id() holds the key */ }
+ }
```

## concurrency-limits
- kind: adopt
- since: ironflow-engine after 2.47.1 (#178)

A worker `concurrency` caps one process, not the runs that share an external resource
across workers. A handler that polls the running runs of a repository or tenant and sleeps
until one finishes, or a client that holds back its `POST /api/v1/runs` for the same
reason, races and burns a slot while it waits. Give the run concurrency groups instead: it
is created at once and stays pending until every group counts fewer running root runs than
its limit, while other runs go ahead. `concurrency_limits` in the body,
`--concurrency-limit GROUP=N` in the CLI. Section: Sub-workflow.

```diff
- while running_runs_for("repo:acme").await? >= 2 {
-     sleep(Duration::from_secs(30)).await;
- }
- engine.enqueue_handler("fix-issue", TriggerKind::Api, payload).await?;
+ let options = EnqueueOptions {
+     concurrency_limits: vec![ConcurrencyLimit::new("repo:acme", 2)],
+     ..Default::default()
+ };
+ engine
+     .enqueue_handler_with_options("fix-issue", TriggerKind::Api, payload, options)
+     .await?;
```

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
+ let outcome = ctx
+     .workflow_with(&Collect, input, WorkflowOptions::new().allow_failure())
+     .await?;
+ if let Some(child) = outcome.output().filter(|c| c.status() == RunStatus::Failed) {
+     // child.error() holds the reason
+ }
```

## persistent-environment
- kind: adopt
- since: ironflow-core after 4.8.1, ironflow-engine after 2.47.1 (#175)

Agent steps on `K8sEphemeralProvider` that hand files to each other through a shared,
hand-provisioned PVC (or that redo a clone in every step) can use a persistent
environment instead: each step gets its own claim, created and expired by ironflow, and
a later step resumes it by id. The worker needs `create`, `get`, `patch`, `list` and
`delete` on `persistentvolumeclaims`. Section: Agent, Persistent environment.

```diff
  let provider = K8sEphemeralProvider::sandboxed(&image)
-     .pvc_volume("shared-workspace", "/workspace")
+     .environment_volume(EnvironmentVolume::new("/workspace"))
      .working_dir("/workspace");

- ctx.agent("fix", AgentStepConfig::new("Fix the failing test")).await?;
+ if let Some(environment) = clone.environment_id.as_deref() {
+     let config = AgentStepConfig::new("Fix the failing test").resume_environment(environment);
+     ctx.agent("fix", config).await?;
+ }
```

## run-output
- kind: adopt
- since: ironflow-engine after 2.47.0 (#164)

A parent that needs a result from its child used to list the child's steps by run id and
pick one by name, or the child wrote the result to an artifact or an external store. The
step name becomes a contract nobody checks, and a renamed step turns the result into
nothing. Let the child publish a typed value with `ctx.set_output(&value)` and read it in
the parent with `child.output::<T>()`: `Ok(None)` when the child set nothing, an error
when the value is not a `T`. The value is persisted on the run (`Run.output`, also on a
failed run), shown by the API and the dashboard, and recorded in the parent's `Workflow`
step so a resume reads the same value. Section: Sub-workflow.

```diff
  // In the child:
+ ctx.set_output(&Verdict { approved })?;
  // In the parent:
  let child = ctx.workflow(&Review, ReviewInput { mr }).await?;
- let steps = ctx.store().list_steps(child.run_id()).await?;
- let verdict = steps.iter().find(|s| s.name == "verdict").map(StepOutput::from);
+ let verdict: Option<Verdict> = child.output()?;
```

## agent-with-meta
- kind: adopt
- since: ironflow-engine after 2.49.0 (#180)

A step that needs a typed answer and the `environment_id` of the same step used to run
without `.output::<T>()` to read the id from the raw `StepOutput`, then parse the text by
hand. `ctx.agent_with_meta` returns the typed `answer` with `environment_id` and
`account_id`. Section: Agent, Persistent environment.

```diff
- let triage = ctx.agent("triage", AgentStepConfig::new("Name the failing test")).await?;
- let environment = triage.environment_id.clone();
+ let reply = ctx
+     .agent_with_meta("triage", AgentStepConfig::new("Name the failing test").output::<Triage>())
+     .await?;
+ let environment = reply.environment_id.clone();   // reply.answer is the Triage
```
