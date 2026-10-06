# Handler code

Changes to the code a workflow author writes. The compiled, current form of every step
lives in `workflow/references/steps.md` (section named in each entry).

## step-output-accessors
- kind: idiom
- since: ironflow-engine 2.39.0 (#105)
- detect: `\.output(\.get\(|\[")`
- detect: `\.get\("(stdout|stderr|exit_code|body)"\)`
- detect: `\.as_str\(\)\.unwrap_or\(`

Still compiles, but a typo in the key becomes `""` in silence. Read a step through its
typed accessors; a stored step through `StepOutput::from(&step)`.

```diff
- let out = log.output.get("stdout").and_then(|v| v.as_str()).unwrap_or("");
+ let out = log.stdout();               // also stderr(), exit_code(), status(), body(), text(), error()
- s.output.as_ref().and_then(|o| o.get("stdout")).and_then(|v| v.as_str())
+ StepOutput::from(&s).stdout()          // use ironflow_engine::executor::StepOutput;
```

## typed-input
- kind: idiom
- since: ironflow-engine 2.23.0
- detect: `from_value.*payload`
- detect: `payload\(\)\.await\?(\[|\.get\()`

```diff
- let payload = ctx.payload().await?;
- let env = payload["env"].as_str().unwrap_or("staging");
+ let input: DeployInput = ctx.input().await?;   // #[derive(Deserialize, JsonSchema)]
```

`ctx.payload()` stays legitimate for a webhook payload the handler does not own.

## input-schema
- kind: idiom
- since: ironflow-engine 2.23.0
- detect: `"properties"`
- detect: `json!\(\{ *"type": *"object"`

```diff
- fn input_schema(&self) -> Option<Value> { Some(json!({"type": "object", "properties": {..}})) }
+ fn input_schema(&self) -> Option<Value> { Some(input_schema_for::<DeployInput>()) }
```

With `TypedWorkflow`: `Self::typed_input_schema()`. Section: Handler metadata.

## approvers
- kind: breaking
- since: ironflow-engine 2.39.0 (#105)
- detect: `ApprovalRule|with_rule\(`
- compiler: `unresolved import ... ApprovalRule`, `no method named with_rule`

Rules were expressions in strings, evaluated at run time. The handler now decides in Rust.

```diff
- ApprovalConfig::new("Release the payment?")
-     .with_rule(ApprovalRule::new("payload.amount > 10000", 2).with_approver_groups(["finance"]))
+ let payment: Payment = ctx.input().await?;
+ let approvers = if payment.amount > 10_000 {
+     Approvers::at_least(2).from_groups(["finance"]).because("amount > 10k")
+ } else {
+     Approvers::any()
+ };
+ ApprovalConfig::new("Release the payment?").requiring(approvers)
```

A rule on `labels.*` or `steps.*` becomes a value the handler already has. Section:
Approval, Several approvers.

## when-typed
- kind: breaking
- since: ironflow-engine 2.39.0 (#105)
- detect: `\.when\("[^"]*(==|!=|>|<)`
- detect: `[|]p[|] *p\[`
- compiler: `type annotations needed`, `cannot index into a value of type`

```diff
- if ctx.when("input.env == 'prod'", |p| p["env"] == "prod").await? {
+ if ctx.when("production run", |i: &DeployInput| i.env == Env::Prod).await? {
```

The label is a name for the operator, never parsed. A payload that does not match the type
now fails the run instead of taking the `else` branch. Section: Conditions.

## decision-derive
- kind: breaking
- since: ironflow-engine 2.39.0 (#105)
- detect: `\.(noul|choice|score)\("`
- compiler: `no method named noul` / `choice` / `score`

```diff
- DecisionConfig::new(state)
-     .noul("is_urgent", "Does this convey urgency?")
-     .choice("team", "Which team?", &["billing", "technical"])
- let team = &out.choice("team")?.choice;
+ #[derive(DecisionChoice)] enum Team { Billing, Technical }
+ #[derive(DecisionAnswers)] struct Triage {
+     #[noul("Does this convey urgency?")] is_urgent: f64,
+     #[choice("Which team?")] team: Team,
+ }
+ let triage = ctx.decision("triage", DecisionConfig::new(state).answers::<Triage>()).await?;
+ match triage.team { Team::Billing => .., Team::Technical => .. }
```

Option labels are the variants in `snake_case`: `#[choice(rename = "..")]` keeps an old
label the decision provider was tuned on. Section: Decision.

## agent-typed-output
- kind: breaking
- since: ironflow-engine 2.39.0 (#105)
- detect: `\.json::<`
- compiler: `no method named json found for struct <YourType>`

Only the hits that follow an agent step built with `.output::<T>()`: the step now returns
the `T`. A `json::<T>()` on an HTTP body or an operation output is correct.

```diff
- let out = ctx.agent("review", config.output::<Review>()).await?;
- let review: Review = out.json()?;
+ let review = ctx.agent("review", config.output::<Review>()).await?;
```

## tool-enum
- kind: breaking
- since: ironflow-engine 2.39.0 (#105)
- detect: `allow_tool\("`
- compiler: `mismatched types: expected Tool, found &str`

```diff
- .allow_tool("Bash").allow_tool("mcp__gitlab__get_issue")
+ .allow_tool(Tool::Bash).allow_tool(Tool::Custom("mcp__gitlab__get_issue".into()))
```

`use ironflow_engine::config::Tool;`. Section: Agent.

## artifact-handles
- kind: breaking
- since: ironflow-engine 2.39.0 (#105)
- detect: `get_artifact\("`
- detect: `\.input(_at)?\("[^"]*", *"`
- compiler: `this method takes 1 argument but 2 arguments were supplied`

```diff
- ShellConfig::new("./publish").input("build", "report.html")
- let bytes = ctx.get_artifact("build", "report.html").await?;
+ let report = build.artifact("report.html")?;   // `build` = the producing step's output
+ ShellConfig::new("./publish").input(&report)
+ let bytes = ctx.get_artifact(&report).await?;
- ctx.put_artifact(step_id, "summary.json", None, bytes)
+ ctx.put_artifact(&summary_output, "summary.json", None, bytes)
```

Section: Artifacts.

## sub-workflow
- kind: breaking
- since: ironflow-engine 2.39.0 (#105)
- detect: `workflow\(&[A-Za-z_]+, *json!`
- detect: `workflow_dyn\(`
- detect: `vec!\[ *"[a-z0-9]+-[a-z0-9-]+"\.(to_string|into)\(\)`
- compiler: `the trait bound Child: TypedWorkflow is not satisfied`

```diff
+ impl TypedWorkflow for Collect { type Input = CollectInput; }   // on the child
- let res = ctx.workflow(&Collect, json!({"scope": "disk"})).await?;
- let child = res.output.get("run_id").and_then(|v| v.as_str()).unwrap_or("unknown");
+ let child = ctx.workflow(&Collect, CollectInput { scope: "disk".into() }).await?;
+ let steps = ctx.store().list_steps(child.run_id()).await?;
- fn sub_workflows(&self) -> Vec<String> { vec!["collect".to_string()] }
+ fn sub_workflows(&self) -> Vec<String> { sub_workflow_names(&[&Collect]) }
```

A child without input: `type Input = ();`, called with `()`. `workflow_dyn` is
deprecated, kept for a child only known at run time. Section: Sub-workflow.

## read-file-tool
- kind: deprecated
- since: ironflow-core 4.3.1 (#115)
- detect: `ReadFileTool::new\(`

`new()` reads anywhere the worker can. Restrict it, or say it out loud.

```diff
- ReadFileTool::new()
+ ReadFileTool::with_allowed_paths(vec![PathBuf::from("/srv/repos")])   // a missing root panics
+ ReadFileTool::unrestricted()                       // only if full access is intended
```

## mcp-tool-filter
- kind: breaking
- since: ironflow-core 4.0.0 (#116)
- detect: `McpToolFilter::new\(`
- compiler: `no function or associated item named new found for struct McpToolFilter`

```diff
- McpToolFilter::new().allow(&["list_incidents", "get_incident"])
+ McpToolFilter::allow(["list_incidents", "get_incident"])
- McpToolFilter::new().require_read_only_hint()
+ McpToolFilter::read_only()
```

A listed tool the server does not expose now fails with `McpError::ToolNotFound` instead of
being dropped. `McpError` is `#[non_exhaustive]`: a `match` on it needs a `_` arm.

## model-aliases
- kind: behavior
- since: ironflow-core 3.0.0
- detect: `Model::(OPUS|SONNET)([^_A-Z0-9]|$)`
- detect: `model\("(opus|sonnet)"\)`

The `opus` and `sonnet` aliases resolve to the Claude 5 family; the Anthropic API adapter
defaults to `claude-sonnet-5`. Cost and output change without a code change. Keep the alias
(recommended), or pin a full model id string per step to keep the old model.

## http-internal-hosts
- kind: behavior
- since: ironflow-core after 4.8.0, ironflow-engine after 2.44.0 (#155)
- detect: `HttpConfig::(get|post|put|patch|delete)\("https?://(localhost|127\.|10\.|192\.168\.|[^"/]*\.svc[.:/"])`

`ctx.http` and `Http` refuse a host that resolves to a private, loopback, link-local or
cloud metadata address, not only an IP literal: `localhost`, a Kubernetes service name or
an internal DNS name now fail before anything is sent, without retries. The step fails with
`URL host <name> resolves to a blocked IP address`. Allow each internal service the step
must reach, or every one of the deployment through the worker's
`IRONFLOW_HTTP_ALLOWED_HOSTS` (comma-separated). Proxy variables are ignored for hosts that
are not allowed. A test serving a stub on `127.0.0.1` needs the same allowance.

```diff
- ctx.http("invoices", HttpConfig::get("http://billing.internal:8080/invoices")).await?;
+ ctx.http(
+     "invoices",
+     HttpConfig::get("http://billing.internal:8080/invoices").allow_host("billing.internal"),
+ )
+ .await?;
```

## sub-workflow-suspension
- kind: behavior
- since: ironflow-engine after 2.44.1 (#160)

A child called through `ctx.workflow` may now suspend on an approval, a human input, a
signal wait or a delay. The child run keeps its status, the parent's `Workflow` step stays
`Running`, and the parent and every ancestor suspend with it. Resolving the child (answer,
approval, signal, elapsed delay) resumes the root run, which re-enters the same child run;
rejecting an approval inside the child fails every ancestor. A handler that kept gates out
of its children, or copied them into the parent to avoid a failed step, can move them back.

```diff
- // Gate kept in the parent: a child could not suspend.
- ctx.approval("release", ApprovalConfig::new("Release?")).await?;
- ctx.workflow(&Release, ReleaseInput { version }).await?;
+ // `Release` holds its own `ctx.approval("release", ..)`.
+ ctx.workflow(&Release, ReleaseInput { version }).await?;
```

## cancel-cascades-to-children
- kind: behavior
- since: ironflow-engine after 2.48.2 (#169)
- detect: `child run .* is Cancelled`

Cancelling a run cancels its active sub-workflow runs, and a parent that fails or retries
cancels the children its attempt left running, which releases their concurrency keys. A
child cancelled directly fails its parent's step with `EngineError::ChildRunCancelled`
(never retried) instead of `InvalidWorkflow("child run .. is Cancelled")`; with
`allow_failure` the step completes with a `Cancelled` child. The SDK's `cancel_run`
returns a `CancelRunResponse` (the run's fields plus `cancelled_descendants`). Code that
cancelled children one by one, or cleared stuck children to free a key, can drop it.

```diff
- for child in stuck_children { client.cancel_run(child).await?; }
- client.cancel_run(root).await?;
+ let cancelled = client.cancel_run(root).await?.data.cancelled_descendants;
```

## test-result-output
- kind: behavior
- since: ironflow-engine after 2.47.0 (#164)

`TestResult::output()` returns the run output the handler set with `ctx.set_output`, and
`Value::Null` when it set none. It used to return the last step's output, so a test that
read a step through it now sees `Null`. Read the step explicitly instead.

```diff
- assert_eq!(result.output()["stdout"], "compiled");
+ assert_eq!(result.steps().last().expect("a step").step_output().stdout(), "compiled");
```

## capacity-wait
- kind: behavior
- since: ironflow-core after 4.9.1, ironflow-engine after 2.48.3, ironflow-worker after 2.23.18 (#184)
- detect: `no provider account available`

When every Provider Account an agent step may use is rate limited, the run no longer
fails with `AgentError::ProcessFailed` ("no provider account available ..."): it sleeps
until the earliest reset and the step runs again from the start when it wakes, for up to
6 hours of cumulative wait. A step still fails when the reset is further away, now with
`AgentError::NoCapacity { next_reset, .. }`. A rejection reported while the step runs
fails it over to the next account, and a rate limit on the worker's own token sleeps
the same way. A step can target `.account(name)` (never fails over, unknown name gives
`AgentError::AccountNotFound`) or `.account_pool(tag)`. Code that matched the old stderr,
or a retry loop around agent steps, can match the typed variant or drop the loop;
`.fail_fast_on_capacity()` restores the fail-fast behavior per step (`Duration::ZERO` on the worker).

```diff
- Err(EngineError::Operation(OperationError::Agent(AgentError::ProcessFailed { stderr, .. })))
-     if stderr.contains("no provider account available") => notify_limited().await?,
+ Err(EngineError::Operation(OperationError::Agent(AgentError::NoCapacity { next_reset, .. }))) =>
+     notify_limited(next_reset).await?,
```

```diff
- let triage = AgentStepConfig::new("Triage the incident");
+ let triage = AgentStepConfig::new("Triage the incident").fail_fast_on_capacity();
```

## agent-session-resume
- kind: behavior
- since: ironflow-core after 4.10.0, ironflow-engine after 2.49.0 (#168)

An agent step interrupted by a lost worker lease no longer starts over on the next
execution: on a Claude Code transport it resumes the session it was running in, with
`DEFAULT_RESUME_PROMPT` or the step's `.resume_prompt(..)`, and keeps the work already
done. A missing session (other machine, ephemeral pod) falls back to the original prompt.
Code that re-ran the whole agent by hand after an interruption, or wrote a "check what is
already done" preamble in the prompt for that case, can drop it and set a resume prompt.
A sandboxed `K8sEphemeralProvider` keeps sessions across pods with `.sessions_volume(claim)`.
A `retry_policy` retry of an agent step now sends the original prompt into the session
the failed try created, so the agent sees what that try did.

```diff
- let review = AgentStepConfig::new("Review the diff. If a review is already half written, finish it.");
+ let review = AgentStepConfig::new("Review the diff.")
+     .resume_prompt("You were interrupted. Finish the review where you stopped.");
```
