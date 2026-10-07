# Human Input

A human input step pauses a workflow run until a person submits a typed answer.
Where an [approval gate](approval-gates.md) asks "yes or no?", a human input asks
for data: answers to clarification questions, a choice, a value. The handler gets
the answer back as a Rust type.

## How it works

1. The handler calls `ctx.human_input::<T>()` with a message. `T` derives
   `Deserialize` and `JsonSchema`.
2. The engine records a step of kind `human_input` whose input holds the message
   and the JSON schema of `T` (under the key `schema`).
3. The step and the run move to `AwaitingApproval`, the same status as an
   approval gate, and an `input_required` event is published on the run's event
   stream.
4. A person posts an answer to
   `POST /api/v1/runs/:id/steps/:step_id/input`. The API validates it against the
   stored schema, completes the step and resumes the run: in the API process
   under `ExecutionMode::Local` (the default), or by requeuing it to `Pending`
   for a worker under `ExecutionMode::Workers` (see
   [execution mode](engine-worker.md#execution-mode)).
5. The handler is replayed: completed steps come from cache and
   `human_input` returns the answer, deserialized into `T`.

## Example

```rust,ignore
use ironflow_engine::prelude::*;
use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Deserialize, JsonSchema)]
struct Answers {
    answers: Vec<String>,
}

let answers: Answers = ctx
    .human_input("clarify", HumanInputConfig::new("Answer the clarification questions"))
    .await?;

ctx.shell("plan", ShellConfig::new(&format!("./plan.sh {}", answers.answers.len())))
    .await?;
```

## Configuration

`HumanInputConfig` reuses the approval gate machinery:

```rust,ignore
use std::time::Duration;

HumanInputConfig::new("Which environment should we target?")
    .assigned_to(Assignee::user("alice"))            // Who is expected to answer
    .requiring(Approvers::any().from_groups(["product"])) // Who may answer
    .with_deadline(Duration::from_secs(3600))         // SLA: one hour to answer
    .on_timeout(EscalationPolicy::AutoReject)         // What happens when it expires
```

| Field | Builder | Meaning |
|-------|---------|---------|
| `message` | `HumanInputConfig::new` | Prompt shown to the person answering |
| `assignee` | `assigned_to` | `Assignee::user` / `Assignee::group` expected to answer |
| `approvers` | `requiring` | Groups allowed to answer. The first valid answer wins, whatever the count |
| `deadline_secs` | `with_deadline` / `with_deadline_secs` | SLA window, in seconds |
| `on_timeout` | `on_timeout` | `EscalationPolicy` applied when the deadline fires (defaults to `AutoReject`) |

Who may answer follows the approval rules: an admin, a member of the `requiring`
groups, the assignee, or someone holding a delegation from the assignee. The
person who answered is recorded on the step like an approval vote.

`EscalationPolicy::AutoApprove` has no meaning without a value:
`on_timeout(EscalationPolicy::AutoApprove)` panics, and so does a `Chain`
containing it. `Notify`, `Escalate`, `AutoReject` and `Chain` behave as for
approval gates.

## API

Answer the input with a body matching the schema:

```bash
curl -X POST "$IRONFLOW_URL/api/v1/runs/$RUN_ID/steps/$STEP_ID/input" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"answers": ["staging", "eu-west-1"]}'
```

- `200` returns the run, now `running`.
- `422` with code `INVALID_INPUT` when the body does not match the schema; each
  violation is listed in `error.details.errors`:

  ```json
  {
    "error": {
      "code": "INVALID_INPUT",
      "message": "input does not match the expected schema",
      "details": { "errors": ["3 is not of type \"array\""] }
    }
  }
  ```

- `409` when the input was already answered or rejected.
- `400` when the step is not a human input, or the run is not waiting on it.

Refuse the input, with an optional reason:

```bash
curl -X POST "$IRONFLOW_URL/api/v1/runs/$RUN_ID/steps/$STEP_ID/reject" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"reason": "out of scope for this sprint"}'
```

`POST /api/v1/runs/:id/approve` refuses a run waiting on a human input with a
`400`: approving it would resume the handler without an answer.

The dashboard shows a form for every pending input on the run page. The CLI has
`ironflow-cli run input <run> <step> --value '{..}'` (or `--value-file`) and
`ironflow-cli run reject-input <run> <step> --reason ..`; the MCP server has the
`submit_input` and `reject_input` tools.

## Rejection

A rejected input does not fail the run by itself. The step is marked `Rejected`
with the reason, the run resumes, and `human_input` returns
`EngineError::HumanInputRejected`, so the handler decides what happens next:

```rust,ignore
match ctx.human_input::<Answers>("clarify", config).await {
    Ok(answers) => { /* use the answers */ }
    Err(EngineError::HumanInputRejected { reason, .. }) => {
        // The reason is free text typed by a person: an argument, not a command line.
        ctx.shell("notify", ShellConfig::exec("./notify.sh", &[&reason]))
            .await?;
    }
    Err(err) => return Err(err),
}
```

A handler that propagates the error fails the run; it is never retried
automatically. The run-level `POST /api/v1/runs/:id/reject` still fails the run
outright.

## Replay and retries

- A run resumed without an answer suspends again on the same step; no new step
  is created.
- An answer given before an automatic retry is carried over to the next attempt:
  the person is not asked twice.
- If the handler changed and the stored answer no longer fits `T`, the run fails
  with a step configuration error.

## Events

`input_required` is published on `GET /api/v1/runs/:id/events` when the step
opens. It carries `run_id`, `step_id`, `step_name`, `step_index`, `message` and
`schema`, everything a client needs to render a form.

## Execution plans

Planning never suspends. The step is recorded with kind `human_input`, and `T`
is built from `{}`: a type with `#[serde(default)]` lets the plan continue past
the input. Otherwise the plan stops there with the reason
`human input '<name>' has no answer while planning`.

## Testing

`TestEngine::with_mock_human_input` answers every human input without waiting:

```rust,ignore
use ironflow_engine::testing::{HumanInputOutcome, TestEngine};
use serde_json::json;

let result = TestEngine::new()
    .with_handler(Clarify)
    .with_mock_human_input(|_name, _config| {
        HumanInputOutcome::Provided(json!({"answers": ["staging"]}))
    })
    .run(json!({}))
    .await?;
```

`HumanInputOutcome::reject("reason")` makes the handler receive
`EngineError::HumanInputRejected`. Without the mock, the run ends in
`AwaitingApproval`: write the answer on the step through the store, then call
`TestEngine::resume`.
