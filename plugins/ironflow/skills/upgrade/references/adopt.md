# New features worth adopting

Each entry replaces a workaround. Nothing breaks if the project keeps the workaround, so
propose these one by one, never apply them in bulk. Most of them change the sequence of
steps: see "Steps are replay keys" in the skill before applying.

## agent-tools-and-output
- kind: adopt
- since: ironflow-engine 2.53.0 (#213)

A step that had to read code and then return a typed verdict used to cost two agent
steps: one with tools, one with `.output::<T>()` reformatting the first one's text. One
step does both now (tested with Claude Code 2.1.296 and 2.1.304).

```diff
- let notes = ctx.agent("read", AgentStepConfig::new(&read_prompt).allow_tool(Tool::Read)).await?;
- let verdict = ctx
-     .agent("verdict", AgentStepConfig::new(&format!("Verdict from:\n{}", notes.text())).output::<Verdict>())
-     .await?;
+ let verdict = ctx
+     .agent("verdict", AgentStepConfig::new(&read_prompt).allow_tool(Tool::Read).output::<Verdict>())
+     .await?;
```

Raise `max_turns` by one per round of tool calls. Section: Agent.

## untrusted-prompt
- kind: adopt
- since: ironflow-core after 4.18.1, ironflow-engine after 2.54.3 (#226)

Text the workflow did not write (an issue, a comment, a diff, a CI log) used to go into
the prompt through `format!`, or behind a hand-made delimiter the text could close.
`Prompt::untrusted` puts it in a block marked with a nonce drawn per prompt, neutralizes
any `untrusted` tag inside it, and opens the prompt with an instruction to treat the
block as data (`DEFAULT_UNTRUSTED_INSTRUCTION`, or `.instruction(..)`). The step name does
not change; `RecordReplayProvider` masks the nonce, but a fixture recorded with the old
prompt no longer matches: record it again.

```diff
- let config = AgentStepConfig::new(&format!("Propose labels for this issue:\n\n{}", issue.stdout()));
+ let prompt = Prompt::new().text("Propose labels for this issue:").untrusted(issue.stdout());
+ let config = AgentStepConfig::new(&prompt.to_string());
```

`Prompt` comes from `ironflow_engine::config` (or `ironflow_core::prompt` in library
mode). Section: Agent.

## available-tools
- kind: adopt
- since: ironflow-core after 4.16.0 (#217)
- detect: `disallowed_tools\(\[[^]]*Tool::Bash`

`.disallowed_tools([Tool::Bash])` kept a step from running a shell through `Bash` only:
`Monitor` still runs a read-only command, and so does `Bash` itself when it is merely left
out of `allow_tool` in `dontAsk` (tested with Claude Code 2.1.296 and 2.1.308).
`.available_tools(...)` offers only the listed built-in tools; MCP tools stay offered.

```diff
- let config = AgentStepConfig::new(&prompt).allow_tool(Tool::Read).disallowed_tools([Tool::Bash]);
+ let config = AgentStepConfig::new(&prompt).allow_tool(Tool::Read).available_tools([Tool::Read]);
```

## owner-pod
- kind: adopt
- since: ironflow-core after 4.17.0, ironflow-ops-k8s after 0.1.43 (#218)

A worker killed during a step left its agent pod, `PodRun` pod or `JobRun` Job running
until the lease expired and the run was released, or until the orphan reaper passed: the
agent kept writing to shared state, outside the run's cost. `.owner_pod(name, uid)` ties
them to the worker pod, and the garbage collector deletes them with it. Only when the
worker runs in the namespace of the pods it creates: an invocation fails otherwise.
`release_owned()` at boot covers a container restarted in the same pod. Expose
`metadata.name` and `metadata.uid` to the worker through the downward API first.

```diff
  let provider = K8sEphemeralProvider::sandboxed(&image)
-     .namespace(&namespace);
+     .namespace(&namespace)
+     .owner_pod(&var("POD_NAME")?, &var("POD_UID")?);
+ provider.release_owned().await?;
  let check = PodRun::new(&kube, "check", &image, "cargo test")
-     .namespace(&namespace);
+     .namespace(&namespace)
+     .owner_pod(&var("POD_NAME")?, &var("POD_UID")?);
```

Section: Agent.

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

## run-priority
- kind: adopt
- since: ironflow-engine after 2.49.0 (#171)

Workers used to pick pending runs oldest first only. To let urgent work jump ahead, a
deployment ran a separate worker pool, or a client held back its low priority
`POST /api/v1/runs` while urgent runs were waiting. Give the run a priority from -100 to
100 instead: workers pick the highest first, then the oldest. A handler declares it with
`fn priority(&self) -> i16`, a caller overrides it with `priority` in the body,
`--priority` in the CLI or `EnqueueOptions::priority`, and a schedule and a child
(`WorkflowOptions::priority`) carry their own. A running run is never preempted and a
waiting one is never aged. Section: Sub-workflow.

```diff
- engine.enqueue_handler("hotfix", TriggerKind::Api, payload).await?;
+ let options = EnqueueOptions {
+     priority: Some(80),
+     ..Default::default()
+ };
+ engine
+     .enqueue_handler_with_options("hotfix", TriggerKind::Api, payload, options)
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

## proxied-secret
- kind: adopt
- since: ironflow-core after 4.11.0 (#177)
- detect: `env_from_secret\("[A-Z_]*TOKEN"`

An agent step on `K8sEphemeralProvider` with the auth proxy that gets a GitHub or GitLab
token through `env_from_secret` hands the real token to the pod: anything running in the
agent container can read it. `proxied_secret` (on the step, or on the provider for every
step) keeps it in the auth proxy instead. The pod receives an opaque token in `<env>` and
the relay base `<proxy>/r` in `<env>_URL`; the proxy injects the real secret only for the
hosts of the allowlist. A `SecretInjection::Basic` secret also rewrites `https://<host>/`
for git, so `git clone` works unchanged. The proxy pods need egress to those hosts.
Section: Agent.

```diff
  let config = AgentStepConfig::new("Open the merge request")
-     .env_from_secret("GITLAB_TOKEN", "gitlab-bot", "token");
+     .proxied_secret(ProxiedSecret {
+         name: "GITLAB_TOKEN".to_string(),
+         env: "GITLAB_TOKEN".to_string(),
+         value: var("GITLAB_TOKEN")?,   // read by the worker, never sent to the pod
+         injection: SecretInjection::Basic { username: "oauth2".to_string() },
+         hosts: vec!["gitlab.com".to_string()],
+     });
```

## schedule-policies
- kind: adopt
- since: ironflow-engine after 2.51.2 (#174)
- detect: `CronSchedule::new\(`

A schedule fired in UTC only, at most once after an outage, and stacked runs when the
previous one was still active. Projects shifted the expression by hand for local time
(and again at each daylight saving change), or checked for an active run at the start of
the handler. Set the timezone, the catch-up of missed occurrences and the overlap on the
schedule instead; a catch-up run reads its occurrence from
`TriggerKind::Cron { scheduled_for, .. }`. Section: Handler metadata.

```diff
- CronSchedule::new("0 7 * * *")   // 9:00 in Paris, summer only
+ CronSchedule::new("0 9 * * *")?
+     .with_timezone("Europe/Paris")?
+     .with_catchup(CatchupPolicy::All)        // or Latest (default), Skip
+     .with_overlap(OverlapPolicy::Skip)       // instead of an active-run check in the handler
```

## workflow-waves
- kind: adopt
- since: ironflow-engine after 2.53.0 (#215)

A parent that hands the same child to several subjects (one per agent, repository or
issue) used to call `ctx.workflow` in a loop, one child after the other, or enqueue root
runs and poll them, which loses the parent link, the shared cost cap and the replay.
`ctx.workflows_with` runs the children concurrently as one wave, each with its own input
and `WorkflowOptions`, and returns the outcomes in the order of the entries.
`WorkflowOptions::step_name` names each child's step (by default every child is named
after its handler) and `label` tags its run. The step names change: bump the handler's
`version()`. Section: Sub-workflow (`sub-workflows.md`).

```diff
- for agent in &agents {
-     ctx.workflow(&Delegate, DelegateInput { agent: agent.clone() }).await?;
- }
+ let wave = agents
+     .iter()
+     .map(|agent| {
+         let options = WorkflowOptions::new()
+             .step_name(format!("delegate-{agent}"))
+             .label("team.example/agent", agent.as_str());
+         (DelegateInput { agent: agent.clone() }, options)
+     })
+     .collect();
+ let outcomes = ctx.workflows_with(&Delegate, wave).await?;
```

## pod-run-exit-stdin
- kind: adopt
- since: ironflow-ops-k8s after 0.1.41 (#216)
- detect: `\.phase == "Failed"`

A `PodRun` script that needed an input, its exit code, or to tell a deadline from a
failure used to wrap its command in `sh`: the input in an env var piped to the command
(refused by Linux past 128 KiB), the exit code printed on a marked last line of the logs,
and a `Failed` pod without that line taken for a timeout. `PodRunOutput` now carries
`exit_code` and `reason` (`DeadlineExceeded` for a pod killed by `active_deadline_seconds`,
with no `exit_code`), `.stdin(input)` feeds the command up to 255 KiB, and `.sandboxed()`
replaces the hand-set hardening options (and adds a read-only root, so the script writes
only to `/tmp` and its volumes). `logs` still interleaves stdout and stderr: keep
`2>/dev/null` in the command to parse stdout alone. A test building `PodRunOutput { .. }`
by hand gains the two fields.

```diff
- let wrapped = format!("printf '%s' \"$INPUT\" | {cmd} 2>/dev/null; printf '\\n{marker} %s\\n' \"$?\"");
- let out = ctx.operation("script", &PodRun::new(&kube, name, image, &wrapped)
-     .env("INPUT", &input)
-     .security(spec)
-     .allow_privilege_escalation(false)
-     .automount_service_account_token(false)
-     .active_deadline_seconds(deadline)).await?;
- let timed_out = out.phase == "Failed" && !out.logs.contains(&marker);
+ let out = ctx.operation("script", &PodRun::new(&kube, name, image, &format!("{cmd} 2>/dev/null"))
+     .stdin(input)
+     .security(spec)   // keeps its uid; without it, a sandboxed pod runs as 10001
+     .sandboxed()
+     .active_deadline_seconds(deadline)).await?;
+ let timed_out = out.reason.as_deref() == Some("DeadlineExceeded");
+ let code = out.exit_code;
```

## pod-up
- kind: adopt
- since: ironflow-ops-k8s after 0.1.45 (#227)
- detect: `readiness_probe = Some\(Probe`

A service pod that had to outlive its step (a headless browser, a database) used to be a
hand-written `Operation`: build the manifest with `PodRun::build_pod`, set the
`readinessProbe` on its container, create the pod, poll `status.conditions` for `Ready`
and `podIP` until a deadline, then read the tail of its log. `PodUp` does it and keeps the
pod under the ironflow cleanup: `lifetime` sets its `ironflow.io/expires-at`. A pod that
ends or is not ready in time fails the step, with the reason and the last 40 lines of its
log in the error, and is deleted: a workaround that returned `ready: false` and let the
handler decide now gets an error to handle instead. As before, a new execution of the run
releases the pod while the step replays its IP: no approval, signal or human input
between the `PodUp` and the steps that use it.

```diff
- let mut pod = PodRun::new(&kube, name, image, &script).label(LABEL_RUN_ID, &run_id)
-     .timeout(lifetime).active_deadline_seconds(lifetime + margin).build_pod();
- pod.spec.as_mut().unwrap().containers[0].readiness_probe = Some(Probe { exec, ..Default::default() });
- let out = ctx.operation("runtime-up", &RuntimeUp { pods, pod, ready_timeout }).await?.json::<RuntimeUpOutput>()?;
- let ip = if out.ready { out.ip.unwrap() } else { return Err(..) };
+ let run = PodRun::new(&kube, name, image, &script).label(LABEL_RUN_ID, &run_id)
+     .active_deadline_seconds(lifetime + margin);
+ let up = PodUp::new(run, lifetime)
+     .readiness_probe(Probe { exec, ..Default::default() })
+     .ready_timeout(ready_timeout);
+ let ip = ctx.operation("runtime-up", &up).await?.json::<PodUpOutput>()?.ip;
```

## gitlab-secret-masking
- kind: adopt
- since: ironflow-ops-gitlab after 0.1.41 (#225)

A handler that published agent or command output to GitLab used to mask secrets itself
before every write (note, issue, merge request), and a new write site that forgot it
published the secret. `gitlab.op(endpoint)` now masks the request body: the client's own
token, the values added with `GitLab::with_secrets`, and `sk-ant-` / `glpat-` tokens.
`op.masks_secrets()` tells whether anything is masked, for a warning log.

The project's own masking function (`mask` below) and its calls can go; keep the list of
secret values it read, and pass it to `with_secrets`.

```diff
- let gitlab = GitLab::new(&token, host).await?;
- let body = mask(&body, &secret_values);
+ let gitlab = GitLab::new(&token, host).await?.with_secrets(&secret_values);
  let endpoint = CreateMergeRequestNote::builder()
      .project(project_id)
      .merge_request(mr_iid)
      .body(body)
      .build()?;
- ctx.operation("notify-mr", &gitlab.op(endpoint)).await?;
+ let op = gitlab.op(endpoint);
+ if op.masks_secrets() {
+     warn!(step = "notify-mr", "secret masked in text published to GitLab");
+ }
+ ctx.operation("notify-mr", &op).await?;
```

## pod-run-scratch-tolerations
- kind: adopt
- since: ironflow-core after 4.19.0, ironflow-ops-k8s after 0.1.47 (#228)

A `PodRun` that needed a writable scratch space built the `Volume` and `VolumeMount` by
hand, and a toleration list read from a config string was parsed by hand once for the agent
providers and converted again to `k8s_openapi` for the pod. `.scratch_volume(..)` adds a
bounded `emptyDir` (no storage class) or an ephemeral claim deleted with the pod, and
`K8sToleration::parse_list` parses `key[=value]:Effect` entries once for both the providers
and `PodRun::k8s_tolerations`.

```diff
- let run = PodRun::new(&kube, name, image, cmd)
-     .volume(
-         Volume { name: "scratch".into(), empty_dir: Some(EmptyDirVolumeSource {
-             size_limit: Some(Quantity("10Gi".into())), ..Default::default() }), ..Default::default() },
-         VolumeMount { name: "scratch".into(), mount_path: "/scratch".into(), ..Default::default() },
-     )
-     .toleration(Toleration { key: Some(key), operator: Some("Equal".into()), value: Some(value),
-         effect: Some("NoSchedule".into()), ..Default::default() });
+ let tolerations = K8sToleration::parse_list(&cfg.tolerations)?;
+ let run = PodRun::new(&kube, name, image, cmd)
+     .scratch_volume("scratch", "/scratch", "10Gi", None)
+     .k8s_tolerations(tolerations);
```

## agent-role
- kind: adopt
- since: ironflow-engine after 2.55.1
- detect: `fn [a-z_]+\([^)]*\) -> (AgentStepConfig|AgentConfig)`

A function that returns an `AgentStepConfig` to share an agent between workflows does not
lock anything: each caller can change the persona, the tools or the permission mode, and
the output type is repeated at every call. An `impl AgentRole` defines the persona, tools,
rights and output type once; the caller can only override the model, budget, turn limit,
effort and per-call settings. Section: Agent.

```diff
- fn reviewer(diff: &str) -> AgentStepConfig {
-     AgentStepConfig::new(&Prompt::new().text("Review this diff:").untrusted(diff).to_string())
-         .append_system_prompt("You are a strict reviewer.")
-         .allow_tool(Tool::Read)
-         .output::<Review>()
- }
- let review = ctx.agent("review", reviewer(&diff)).await?;
+ struct Reviewer;
+ impl AgentRole for Reviewer {
+     type Input = String;
+     type Output = Review;
+     fn prompt(&self, input: String) -> String {
+         Prompt::new().text("Review this diff:").untrusted(&input).to_string()
+     }
+     fn append_system_prompt(&self) -> Option<&str> { Some("You are a strict reviewer.") }
+     fn tools(&self) -> Vec<Tool> { vec![Tool::Read] }
+ }
+ let review = ctx.agent("review", Reviewer.task(diff)).await?;
```

## agent-role-cost-cap
- kind: adopt
- since: ironflow-engine after 2.56.0, ironflow-core after 4.20.0 (#234)

`AgentRole` gained `cost_cap()`. A role whose spend had to be bounded across runs was
tracked by hand: summing the cost of its steps from the store, or a per-run cap
(`max_cost_usd`) that does not bound the role over a month. A role with a `RoleCostCap` is
charged, under its `name()` (the type name by default; override it to keep the counter
across a type rename), to one counter shared by every run of every workflow, over a
calendar UTC day or month; once it is reached, the step fails with
`AgentError::RoleCostCapReached` before the provider is called. Give its tasks a
`max_budget_usd`: a step without one holds all the room left while it runs. Section: Agent.

```diff
  impl AgentRole for Reviewer {
      type Input = String;
      type Output = Review;
      fn prompt(&self, input: String) -> String { input }
+     fn name(&self) -> &str { "reviewer" }
+     fn cost_cap(&self) -> Option<RoleCostCap> {
+         Some(RoleCostCap::per_month(Decimal::new(50, 0)))
+     }
  }
- let spent = role_cost_this_month(&store, "reviewer").await?;
- if spent >= Decimal::new(50, 0) {
-     return Err(EngineError::StepConfig("reviewer budget exhausted".into()));
- }
  let review = ctx.agent("review", Reviewer.task(diff).max_budget_usd(0.50)).await?;
```
