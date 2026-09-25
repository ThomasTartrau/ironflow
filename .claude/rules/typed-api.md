---
paths:
  - "**/*.rs"
  - "plugins/**/*.md"
  - "docs/book/**/*.md"
---

# Typed workflow-author API

Workflow code, examples, the mdBook and the Claude Code plugin use the typed API. A typo
must fail to compile, or fail loudly at run time; it must never turn into `null`, `false`
or `""` in silence. Do not reintroduce the patterns below.

## Forbidden

| Pattern | Use instead |
|---|---|
| An expression in a string: `"input.env == 'prod'"`, `"payload.amount > 10000"` | Plain Rust on the typed input: `ctx.when("production run", \|i: &DeployInput\| i.env == Env::Prod)`, a `match` that builds `Approvers::at_least(n).from_groups([..])` |
| A `Value` read by key: `output["stdout"]`, `.get("run_id")`, `\|p\| p["env"]`, `.as_str().unwrap_or(..)` on a step output | `stdout()`, `stderr()`, `exit_code()`, `status()`, `body()`, `text()`, `json::<T>()`, `ctx.input::<T>()`, `SubWorkflowOutput::run_id()`, `StepOutput::from(&step)` for a stored step |
| A step name copied by hand to reach what it produced: `input("build", "report.html")`, `get_artifact("build", ..)`, a step id looked up for `put_artifact` | The handle the producing step gives out: `build.artifact("report.html")?`, `ctx.put_artifact(&output, ..)` |
| Decision answers read by key: `out.choice("team")?.choice`, `out.noul("x")?` | A `#[derive(DecisionAnswers)]` struct and `#[derive(DecisionChoice)]` enum, `DecisionConfig::new(state).answers::<T>()` |
| `out.json::<T>()` right after `ctx.agent(.., config.output::<T>())` | Nothing: `ctx.agent` already returns the `T` |
| Tool names as strings: `"Bash"`, `"Read"` | `Tool::Bash`, `Tool::Read`, `Tool::Custom("mcp__server__tool".into())` |
| A sub-workflow payload built with `json!({..})` | `impl TypedWorkflow for Child { type Input = ChildInput; }` and `ctx.workflow(&Child, ChildInput { .. })` |
| Handler names as strings in `sub_workflows()` | `sub_workflow_names(&[&Child])` |
| A JSON Schema written by hand | `input_schema_for::<T>()`, or `Self::typed_input_schema()` with `TypedWorkflow` |

## Strings that stay strings

They are data, not code: shell commands, URLs, headers, run labels, approver groups, step
names (the first argument of every `ctx.*` method), messages and prompts, decision
instructions, the branch label of `ctx.when` / `ctx.when_dynamic`, approval reasons
(`Approvers::because`), artifact file names.

## Where a `Value` is accepted

- The raw run payload, `ctx.payload()`, for a handler driven by a webhook it does not own.
- `DecisionConfig::new(state)`: the state is any serializable value.
- `Operation::execute` returns a `Value`; the handler reads it with `json::<T>()` into a struct.
- `ctx.workflow_dyn`, deprecated, for a child only known at run time.
- Tests of the engine, API and store themselves, which assert on the persisted format.
