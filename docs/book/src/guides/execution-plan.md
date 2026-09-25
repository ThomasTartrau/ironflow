# Execution Plans

An execution plan answers one question before you trigger anything: *what would
this workflow do with this input?* It lists the steps the run would create, in
order, with their dependencies, their parallel waves, their branch conditions
and — when the workflow has a history — an estimate of how long each one takes.

## What plan mode does, and does not

Ironflow workflows are Rust-native handlers, not declarative graphs. There is no
static file to read, so the only way to know which steps a run would create is to
execute the handler with every step method short-circuited. That is plan mode.

In plan mode:

- no shell command is spawned, no HTTP request is sent, no agent is called;
- no custom [`Operation`](../concepts/operations.md) is executed, so no
  third-party API is touched;
- an [approval gate](../concepts/approval-gates.md) is recorded and stepped
  over instead of suspending the run;
- a delay is recorded without sleeping;
- nothing is written: no run, no step, no dependency, no artifact.

Each step method returns a synthetic, **success-shaped** output instead
(`exit_code: 0` for a shell step, `status: 200` for an HTTP step). A handler
that branches on `build.is_success()` therefore follows the happy path: a plan
shows the nominal branch, not every branch the run might take.

## From the CLI

```bash
ironflow run plan deploy --input '{"env":"prod"}'
```

```text
workflow deploy  estimated ~2m 14s
├─ build [shell] ~1m 02s
├─ parallel-1
  ├─ test-unit [shell] ~41s
  ├─ test-integration [shell] ~58s
  ├─ lint [shell] ~12s
└─ deploy-prod [shell] ~14s (when production run = true)
```

Useful flags:

| Flag | Meaning |
|------|---------|
| `--input '<json>'` | Input payload, inline. Must be a JSON object. |
| `--input-file <path>` | Input payload read from a file. Mutually exclusive with `--input`. |
| `--max-depth <n>` | How deep sub-workflows are expanded. Defaults to 3, at most 10. |
| `--no-estimates` | Skip the duration estimate query against run history. |
| `--json` | Emit the raw plan instead of the tree. |

## From the API

```http
POST /api/v1/workflows/{name}/plan
```

```json
{
  "payload": { "env": "prod" },
  "max_depth": 3,
  "estimate_durations": true
}
```

The response is the usual envelope around an execution plan:

```json
{
  "data": {
    "workflow": "deploy",
    "max_depth": 3,
    "truncated": false,
    "estimated_duration_ms": 134000,
    "steps": [
      {
        "name": "build",
        "kind": "shell",
        "workflow": "deploy",
        "depth": 0,
        "depends_on": [],
        "estimated_duration_ms": 62000
      },
      {
        "name": "deploy-prod",
        "kind": "shell",
        "workflow": "deploy",
        "depth": 0,
        "depends_on": ["build"],
        "condition": {
          "state": "evaluated",
          "expression": "production run",
          "value": true
        }
      }
    ]
  }
}
```

The route needs an authenticated caller, like `GET /api/v1/workflows/{name}`:
planning has no side effect, so it is not admin-only. It answers `400` for a
`max_depth` outside `1..=10` or a payload that is not a JSON object, and `404`
for a workflow that is not registered.

## Conditions

A condition appears on a step in one of three states:

| State | Meaning |
|-------|---------|
| `evaluated` | Declared with [`ctx.when`](../concepts/steps.md#conditions) and resolved against the input. |
| `skipped` | The handler called `ctx.skip(name, reason)` on this branch. |
| `unevaluable` | Declared with `ctx.when_dynamic`: it depends on a step output, unknown before the run. |

A plain Rust `if` on a step output stays invisible to the planner by
construction. `ctx.when_dynamic` exists precisely to surface it.

## Durations

When `estimate_durations` is on (the default), the planner samples the most
recent completed runs of the workflow and averages each step's duration by name.
A step with no history carries no estimate, and a workflow with no history at
all reports `estimated_duration_ms` as absent rather than zero. The plan total
sums sequential steps and counts each parallel wave once, at its slowest member.

## Sub-workflows and limits

`ctx.workflow(...)` is expanded in place: the child's steps appear inline with
`depth` incremented and `workflow` set to the child's name. `max_depth` bounds
that expansion, so a workflow that invokes itself stops there.

Two guards keep a plan bounded, and both set `truncated` with an
`incomplete_reason`:

- the depth limit above;
- a hard cap of 1000 planned steps, which stops a handler that loops.

A handler that returns an error mid-plan does not fail the request either: the
partial plan is returned, with `incomplete_reason` carrying the error.

## Limits worth knowing

- Synthetic outputs are success-shaped, so only the nominal branch is planned.
- A handler that unwraps a decision answer, or deserializes `ctx.input::<T>()`
  against a payload it does not match, aborts the plan; you get the steps
  recorded so far plus the reason.
- Secrets and artifacts are not resolved while planning.
